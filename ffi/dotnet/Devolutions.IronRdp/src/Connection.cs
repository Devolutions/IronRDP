using System.Buffers.Binary;
using System.Net;
using System.Net.Security;
using System.Net.Sockets;

namespace Devolutions.IronRdp;

public static class Connection
{
    public static Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string serverName,
        CliprdrBackendFactory? factory, int port = 3389, CancellationToken cancellationToken = default)
    {
        return Connect(config, serverName, factory, null, port, cancellationToken);
    }

    /// <summary>
    /// Connects with Kerberos available to CredSSP when <paramref name="kerberosConfig"/> is set. Kerberos needs
    /// <paramref name="serverName"/> to be the server's host name, because the service ticket is for
    /// <c>TERMSRV/serverName</c>. Without it, CredSSP uses NTLM.
    /// </summary>
    public static async Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string serverName,
        CliprdrBackendFactory? factory, KerberosConfig? kerberosConfig, int port = 3389,
        CancellationToken cancellationToken = default)
    {
        var client = await CreateTcpConnection(serverName, port, cancellationToken);
        string clientAddr = client.Client.LocalEndPoint.ToString();
        System.Diagnostics.Debug.WriteLine(clientAddr);

        var framed = new Framed<NetworkStream>(client.GetStream());

        var connector = ClientConnector.New(config, clientAddr);

        ConnectionHelpers.SetupConnector(connector, config, factory);

        await ConnectBegin(framed, connector, cancellationToken);
        var (serverPublicKey, framedSsl) = await SecurityUpgrade(framed, connector, cancellationToken);
        var result = await ConnectionHelpers.ConnectFinalize(serverName, connector, serverPublicKey, framedSsl, cancellationToken, kerberosConfig);

        return (result, framedSsl);
    }

    private static async Task<(byte[], Framed<SslStream>)> SecurityUpgrade(Framed<NetworkStream> framed,
        ClientConnector connector, CancellationToken cancellationToken)
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
        }, cancellationToken);
        var serverPublicKey = await promise.Task;
        Framed<SslStream> framedSsl = new(sslStream);
        connector.MarkSecurityUpgradeAsDone();

        return (serverPublicKey, framedSsl);
    }

    private static async Task ConnectBegin(Framed<NetworkStream> framed, ClientConnector connector,
        CancellationToken cancellationToken)
    {
        var writeBuf = WriteBuf.New();
        while (!connector.ShouldPerformSecurityUpgrade())
        {
            await SingleSequenceStep(connector, writeBuf, framed, cancellationToken);
        }
    }

    internal static async Task<ClientState> ResolveGenerator(CredsspProcessGenerator generator,
        CancellationToken cancellationToken = default)
    {
        var state = generator.Start();

        while (true)
        {
            if (state.IsSuspended())
            {
                var request = state.NetworkRequestIfSuspended!;
                var protocol = request.Protocol;
                var url = request.Url;
                var data = request.Data;

                if (protocol == NetworkRequestProtocol.Tcp)
                {
                    state = generator.Resume(await SendKdcTcpRequest(new Uri(url), Utils.VecU8ToByte(data), cancellationToken));
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
                    var clientState = state.ClientStateIfCompleted;
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

    // Kerberos over TCP (RFC 4120 section 7.2.2): each message is prefixed with its length as a big-endian u32.
    // A KDC may close the connection after replying, so every request uses its own connection, as sspi does.
    // UNTESTED: the whole reply is returned even when it arrives in several reads; a ticket with a large PAC often
    // exceeds one read. Check this against a real KDC when changing how the reply is read.
    private static async Task<byte[]> SendKdcTcpRequest(Uri url, byte[] data, CancellationToken cancellationToken)
    {
        using var client = new TcpClient();
        await client.ConnectAsync(url.DnsSafeHost, url.Port < 0 ? 88 : url.Port, cancellationToken);
        var stream = client.GetStream();
        await stream.WriteAsync(data, cancellationToken);

        var header = new byte[4];
        await stream.ReadExactlyAsync(header, cancellationToken);
        var length = BinaryPrimitives.ReadUInt32BigEndian(header);
        var response = new byte[checked(4 + (int)length)];
        header.CopyTo(response, 0);
        await stream.ReadExactlyAsync(response.AsMemory(4), cancellationToken);
        return response;
    }

    public static async Task SingleSequenceStep<S, T>(S sequence, WriteBuf buf, Framed<T> framed,
        CancellationToken cancellationToken = default)
        where T : Stream
        where S : ISequence
    {
        buf.Clear();

        var pduHint = sequence.NextPduHint();
        Written written;

        if (pduHint != null)
        {
            byte[] pdu = await framed.ReadByHint(pduHint, cancellationToken);
            written = sequence.Step(pdu, buf);
        }
        else
        {
            written = sequence.StepNoInput(buf);
        }

        if (written.WrittenType == WrittenType.Nothing)
        {
            return;
        }

        // Will throw an exception if the size is not set.
        var size = written.Size.Get();

        var response = new byte[size];
        buf.ReadIntoBuf(response);

        await framed.Write(response);
    }

    static async Task<TcpClient> CreateTcpConnection(String servername, int port, CancellationToken cancellationToken)
    {
        IPAddress ipAddress;

        try
        {
            ipAddress = IPAddress.Parse(servername);
        }
        catch (FormatException)
        {
            IPHostEntry ipHostInfo = await Dns.GetHostEntryAsync(servername, cancellationToken).WaitAsync(TimeSpan.FromSeconds(10), cancellationToken);
            ipAddress = ipHostInfo.AddressList[0];
        }

        IPEndPoint ipEndPoint = new(ipAddress, port);

        TcpClient client = new TcpClient();

        await client.ConnectAsync(ipEndPoint, cancellationToken);

        return client;
    }
}

public static class Utils
{
    public static byte[] VecU8ToByte(VecU8 vecU8)
    {
        var len = vecU8.Size;
        var buffer = new byte[len];
        vecU8.Fill(buffer);
        return buffer;
    }
}
