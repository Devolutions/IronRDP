using System.Net;
using System.Net.Security;
using System.Net.Sockets;

namespace Devolutions.IronRdp;

public static class Connection
{
    public static async Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string serverName,
        CliprdrBackendFactory? factory, int port = 3389)
    {
        var client = await CreateTcpConnection(serverName, port);
        string clientAddr = client.Client.LocalEndPoint.ToString();
        System.Diagnostics.Debug.WriteLine(clientAddr);

        var framed = new Framed<NetworkStream>(client.GetStream());

        var connector = ClientConnector.New(config, clientAddr);

        connector.WithDynamicChannelDisplayControl();
        var dvcPipeProxy = config.DvcPipeProxy;
        if (dvcPipeProxy != null)
        {
            connector.WithDynamicChannelPipeProxy(dvcPipeProxy);
        }

        if (factory != null)
        {
            var cliprdr = factory.BuildCliprdr();
            connector.AttachStaticCliprdr(cliprdr);
        }

        await ConnectBegin(framed, connector);
        var (serverPublicKey, framedSsl) = await SecurityUpgrade(framed, connector);
        var result = await ConnectFinalize(serverName, connector, serverPublicKey, framedSsl);

        return (result, framedSsl);
    }

    private static async Task<(byte[], Framed<SslStream>)> SecurityUpgrade(Framed<NetworkStream> framed,
        ClientConnector connector)
    {
        var (streamRequireUpgrade, _) = framed.GetInner();
        var promise = new TaskCompletionSource<byte[]>();
        var sslStream = new SslStream(streamRequireUpgrade, false, (_, certificate, _, _) =>
        {
            promise.SetResult(certificate!.GetPublicKey());
            return true;
        });
        await sslStream.AuthenticateAsClientAsync(new SslClientAuthenticationOptions()
        {
            AllowTlsResume = false
        });
        var serverPublicKey = await promise.Task;
        Framed<SslStream> framedSsl = new(sslStream);
        connector.MarkSecurityUpgradeAsDone();

        return (serverPublicKey, framedSsl);
    }

    private static async Task ConnectBegin(Framed<NetworkStream> framed, ClientConnector connector)
    {
        var writeBuf = WriteBuf.New();
        while (!connector.ShouldPerformSecurityUpgrade())
        {
            await SingleSequenceStep(connector, writeBuf, framed);
        }
    }


    private static async Task<ConnectionResult> ConnectFinalize(string serverName, ClientConnector connector,
        byte[] serverPubKey, Framed<SslStream> framedSsl)
    {
        var writeBuf2 = WriteBuf.New();
        if (connector.ShouldPerformCredssp())
        {
            await PerformCredsspSteps(connector, serverName, writeBuf2, framedSsl, serverPubKey);
        }

        while (!connector.GetDynState().IsTerminal())
        {
            await SingleSequenceStep(connector, writeBuf2, framedSsl);
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

    private static async Task PerformCredsspSteps(ClientConnector connector, string serverName, WriteBuf writeBuf,
        Framed<SslStream> framedSsl, byte[] serverpubkey)
    {
        var credsspSequenceInitResult = CredsspSequence.Init(connector, serverName, serverpubkey);
        var credsspSequence = credsspSequenceInitResult.GetCredsspSequence();
        var tsRequest = credsspSequenceInitResult.GetTsRequest();
        var tcpClient = new TcpClient();
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

            var pduHint = credsspSequence.NextPduHint();
            if (pduHint == null)
            {
                break;
            }

            var pdu = await framedSsl.ReadByHint(pduHint);
            var decoded = credsspSequence.DecodeServerMessage(pdu);

            // Don't remove, DecodeServerMessage is generated, and it can return null
            if (null == decoded)
            {
                break;
            }

            tsRequest = decoded;
        }
    }

    internal static async Task<ClientState> ResolveGenerator(CredsspProcessGenerator generator, System.Net.Sockets.TcpClient tcpClient)
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

                if (protocol == NetworkRequestProtocol.Tcp)
                {
                    if (null == stream)
                    {
                        url = url.Replace("tcp://", "");
                        var split = url.Split(":");
                        await tcpClient.ConnectAsync(split[0], int.Parse(split[1]));
                        stream = tcpClient.GetStream();
                    }

                    stream.Write(Utils.VecU8ToByte(data));
                    var readBuf = new byte[8096];
                    var readlen = await stream.ReadAsync(readBuf, 0, readBuf.Length);
                    var actuallyRead = new byte[readlen];
                    Array.Copy(readBuf, actuallyRead, readlen);
                    state = generator.Resume(actuallyRead);
                }
                else
                {
                    throw new Exception($"Unimplemented protocol: {protocol}");
                }
            }
            else if (state.IsCompleted())
            {
                try
                {
                    var clientState = state.GetClientStateIfCompleted();
                    return clientState;
                }
                catch (IronRdpException ex)
                {
                    System.Diagnostics.Debug.WriteLine($"[ResolveGenerator] Error getting client state: {ex.Message}");
                    System.Diagnostics.Debug.WriteLine($"[ResolveGenerator] Error kind: {ex.Inner?.Kind}");
                    System.Diagnostics.Debug.WriteLine($"[ResolveGenerator] Stack trace: {ex.StackTrace}");
                    throw;
                }
            }
            else
            {
                var errorMsg = $"[ResolveGenerator] Generator state is neither suspended nor completed. IsSuspended={state.IsSuspended()}, IsCompleted={state.IsCompleted()}";
                System.Diagnostics.Debug.WriteLine(errorMsg);
                throw new InvalidOperationException(errorMsg);
            }
        }
    }

    public static async Task SingleSequenceStep<S, T>(S sequence, WriteBuf buf, Framed<T> framed)
        where T : Stream
        where S : ISequence
    {
        buf.Clear();

        var pduHint = sequence.NextPduHint();
        Written written;

        if (pduHint != null)
        {
            byte[] pdu = await framed.ReadByHint(pduHint);
            written = sequence.Step(pdu, buf);
        }
        else
        {
            written = sequence.StepNoInput(buf);
        }

        if (written.GetWrittenType() == WrittenType.Nothing)
        {
            return;
        }

        // Will throw an exception if the size is not set.
        var size = written.GetSize().Get();

        var response = new byte[size];
        buf.ReadIntoBuf(response);

        await framed.Write(response);
    }

    static async Task<TcpClient> CreateTcpConnection(String servername, int port)
    {
        IPAddress ipAddress;

        try
        {
            ipAddress = IPAddress.Parse(servername);
        }
        catch (FormatException)
        {
            IPHostEntry ipHostInfo = await Dns.GetHostEntryAsync(servername).WaitAsync(TimeSpan.FromSeconds(10));
            ipAddress = ipHostInfo.AddressList[0];
        }

        IPEndPoint ipEndPoint = new(ipAddress, port);

        TcpClient client = new TcpClient();

        await client.ConnectAsync(ipEndPoint);

        return client;
    }
}

public static class Utils
{
    public static byte[] VecU8ToByte(VecU8 vecU8)
    {
        var len = vecU8.GetSize();
        var buffer = new byte[len];
        vecU8.Fill(buffer);
        return buffer;
    }
}
