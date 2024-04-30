
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using Devolutions.IronRdp;

public class Connection
{

    public static async Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string servername, ushort port = 3389)
    {

        var stream = await CreateTcpConnection(servername, port);
        var framed = new Framed<NetworkStream>(stream);

        ClientConnector connector = ClientConnector.New(config);

        var ip = await Dns.GetHostAddressesAsync(servername);
        if (ip.Length == 0)
        {
            throw new IronRdpLibException(IronRdpLibExceptionType.CannotResolveDns, "Cannot resolve DNS to " + servername);
        }

        var socketAddrString = ip[0].ToString() + ":" + port.ToString();
        connector.WithServerAddr(socketAddrString);

        await connectBegin(framed, connector);
        var (serverPublicKey, framedSsl) = await securityUpgrade(servername, framed, connector);
        var result = await ConnectFinalize(servername, connector, serverPublicKey, framedSsl);
        return (result, framedSsl);
    }

    private static async Task<(byte[], Framed<SslStream>)> securityUpgrade(string servername, Framed<NetworkStream> framed, ClientConnector connector)
    {
        byte[] serverPublicKey;
        Framed<SslStream> framedSsl;
        var (streamRequireUpgrade, _) = framed.GetInner();
        var promise = new TaskCompletionSource<byte[]>();
        var sslStream = new SslStream(streamRequireUpgrade, false, (sender, certificate, chain, sslPolicyErrors) =>
        {
            promise.SetResult(certificate!.GetPublicKey());
            return true;
        });
        await sslStream.AuthenticateAsClientAsync(servername);
        serverPublicKey = await promise.Task;
        framedSsl = new Framed<SslStream>(sslStream);
        connector.MarkSecurityUpgradeAsDone();

        return (serverPublicKey, framedSsl);
    }

    private static async Task connectBegin(Framed<NetworkStream> framed, ClientConnector connector)
    {
        var writeBuf = WriteBuf.New();
        while (!connector.ShouldPerformSecurityUpgrade())
        {
            await SingleConnectStep(connector, writeBuf, framed);
        }
    }


    private static async Task<ConnectionResult> ConnectFinalize(string servername, ClientConnector connector, byte[] serverpubkey, Framed<SslStream> framedSsl)
    {
        var writeBuf2 = WriteBuf.New();
        if (connector.ShouldPerformCredssp())
        {
            await PerformCredsspSteps(connector, servername, writeBuf2, framedSsl, serverpubkey);
        }
        while (!connector.GetDynState().IsTerminal())
        {
            await SingleConnectStep(connector, writeBuf2, framedSsl);
        }

        ClientConnectorState state = connector.ConsumeAndCastToClientConnectorState();

        if (state.GetEnumType() == ClientConnectorStateType.Connected)
        {
            return state.GetConnectedResult();
        }
        else
        {
            throw new IronRdpLibException(IronRdpLibExceptionType.ConnectionFailed, "Connection failed");
        }
    }

    private static async Task PerformCredsspSteps(ClientConnector connector, string serverName, WriteBuf writeBuf, Framed<SslStream> framedSsl, byte[] serverpubkey)
    {
        var credsspSequenceInitResult = CredsspSequence.Init(connector, serverName, serverpubkey, null);
        var credsspSequence = credsspSequenceInitResult.GetCredsspSequence();
        var tsRequest = credsspSequenceInitResult.GetTsRequest();
        TcpClient tcpClient = new TcpClient();
        while (true)
        {
            var generator = credsspSequence.ProcessTsRequest(tsRequest);
            var clientState = await ResolveGenerator(generator, tcpClient);
            writeBuf.Clear();
            var written = credsspSequence.HandleProcessResult(clientState, writeBuf);

            if (written.GetSize().IsSome())
            {
                var actualSize = (int)written.GetSize().Get();
                var response = new byte[actualSize];
                writeBuf.ReadIntoBuf(response);
                await framedSsl.Write(response);
            }

            var pduHint = credsspSequence.NextPduHint()!;
            if (pduHint == null)
            {
                break;
            }

            var pdu = await framedSsl.ReadByHint(pduHint);
            var decoded = credsspSequence.DecodeServerMessage(pdu);

            if (null == decoded)
            {
                break;
            }

            tsRequest = decoded;
        }
    }

    private static async Task<ClientState> ResolveGenerator(CredsspProcessGenerator generator, TcpClient tcpClient)
    {
        var state = generator.Start();
        NetworkStream? stream = null;
        while (true)
        {
            if (state.IsSuspended())
            {
                var request = state.GetNetworkRequestIfSuspended()!;
                var protocol = request.GetProtocol();
                var url = request.GetUrl();
                var data = request.GetData();
                if (null == stream)
                {
                    url = url.Replace("tcp://", "");
                    var split = url.Split(":");
                    await tcpClient.ConnectAsync(split[0], int.Parse(split[1]));
                    stream = tcpClient.GetStream();

                }
                if (protocol == NetworkRequestProtocol.Tcp)
                {
                    stream.Write(Utils.Vecu8ToByte(data));
                    var readBuf = new byte[8096];
                    var readlen = await stream.ReadAsync(readBuf, 0, readBuf.Length);
                    var actuallyRead = new byte[readlen];
                    Array.Copy(readBuf, actuallyRead, readlen);
                    state = generator.Resume(actuallyRead);
                }
                else
                {
                    throw new Exception("Unimplemented protocol");
                }
            }
            else
            {
                var client_state = state.GetClientStateIfCompleted();
                return client_state;
            }
        }
    }

    static async Task SingleConnectStep<T>(ClientConnector connector, WriteBuf buf, Framed<T> framed)
    where T : Stream
    {
        buf.Clear();

        var pduHint = connector.NextPduHint();
        Written written;
        if (pduHint != null)
        {
            byte[] pdu = await framed.ReadByHint(pduHint);
            written = connector.Step(pdu, buf);
        }
        else
        {
            written = connector.StepNoInput(buf);
        }

        if (written.GetWrittenType() == WrittenType.Nothing)
        {
            return;
        }

        // will throw if size is not set
        var size = written.GetSize().Get();

        var response = new byte[size];
        buf.ReadIntoBuf(response);

        await framed.Write(response);
    }

    static async Task<NetworkStream> CreateTcpConnection(String servername, int port)
    {
        IPAddress ipAddress;

        try
        {
            ipAddress = IPAddress.Parse(servername);
        }
        catch (FormatException)
        {
            IPHostEntry ipHostInfo = await Dns.GetHostEntryAsync(servername);
            ipAddress = ipHostInfo.AddressList[0];
        }

        IPEndPoint ipEndPoint = new(ipAddress, port);

        TcpClient client = new TcpClient();

        await client.ConnectAsync(ipEndPoint);
        NetworkStream stream = client.GetStream();

        return stream;
    }

}

public static class Utils
{
    public static byte[] Vecu8ToByte(VecU8 vecU8)
    {
        var len = vecU8.GetSize();
        byte[] buffer = new byte[len];
        vecU8.Fill(buffer);
        return buffer;
    }
}
