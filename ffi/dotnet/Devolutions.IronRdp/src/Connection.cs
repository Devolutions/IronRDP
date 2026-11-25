using System.Buffers.Binary;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Security;
using System.Net.Sockets;

namespace Devolutions.IronRdp;

public static class Connection
{
    // Far above real Kerberos replies, which stay in the tens of kilobytes even with large PACs.
    internal const int MaxKdcReplyLength = 1024 * 1024;

    private static readonly HttpClient DefaultKdcProxyClient = new();

    public static Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string serverName,
        CliprdrBackendFactory? factory, int port = 3389, CancellationToken cancellationToken = default)
    {
        return Connect(config, serverName, factory, CredsspPackage.Ntlm, null, port, cancellationToken: cancellationToken);
    }

    /// <summary>
    /// Connects with <paramref name="package"/> for CredSSP; <see cref="CredsspPackage"/> says when each package
    /// uses Kerberos or NTLM. <paramref name="kerberosConfig"/> optionally names the KDC. Kerberos needs
    /// <paramref name="serverName"/> to be the server's host name, because the service ticket is for
    /// <c>TERMSRV/serverName</c>.
    /// </summary>
    /// <param name="kdcProxyClient">Sends KDC proxy (<c>http</c>/<c>https</c>) requests, so the host controls
    /// certificate validation, proxy and timeout. Defaults to a shared client with default settings. Direct
    /// <c>tcp</c> KDC requests are bounded by <paramref name="cancellationToken"/>.</param>
    public static async Task<(ConnectionResult, Framed<SslStream>)> Connect(Config config, string serverName,
        CliprdrBackendFactory? factory, CredsspPackage package, KerberosConfig? kerberosConfig, int port = 3389,
        HttpClient? kdcProxyClient = null, CancellationToken cancellationToken = default)
    {
        var client = await CreateTcpConnection(serverName, port, cancellationToken);
        string clientAddr = client.Client.LocalEndPoint.ToString();
        System.Diagnostics.Debug.WriteLine(clientAddr);

        var framed = new Framed<NetworkStream>(client.GetStream());

        var connector = ClientConnector.New(config, clientAddr);

        ConnectionHelpers.SetupConnector(connector, config, factory);

        await ConnectBegin(framed, connector, cancellationToken);
        var (serverPublicKey, framedSsl) = await SecurityUpgrade(framed, connector, cancellationToken);
        var result = await ConnectionHelpers.ConnectFinalize(serverName, connector, serverPublicKey, framedSsl, cancellationToken, package, kerberosConfig, kdcProxyClient);

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
        CancellationToken cancellationToken = default, HttpClient? kdcProxyClient = null)
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

                var response = protocol switch
                {
                    NetworkRequestProtocol.Tcp => await SendKdcTcpRequest(new Uri(url), Utils.VecU8ToByte(data), cancellationToken),
                    NetworkRequestProtocol.Http or NetworkRequestProtocol.Https =>
                        await SendKdcProxyRequest(kdcProxyClient ?? DefaultKdcProxyClient, new Uri(url), Utils.VecU8ToByte(data), cancellationToken),
                    _ => throw new Exception($"Unimplemented protocol: {protocol}"),
                };
                state = generator.Resume(response);
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

    // Kerberos over TCP (RFC 4120 section 7.2.2): each message is prefixed with its length as a big-endian u32. sspi
    // hands over the request with its prefix and expects the reply with its prefix. A KDC may close the connection
    // after replying, so every request uses its own connection, as sspi does.
    internal static async Task<byte[]> SendKdcTcpRequest(Uri url, byte[] data, CancellationToken cancellationToken)
    {
        try
        {
            using var client = new TcpClient();
            await client.ConnectAsync(url.DnsSafeHost, url.Port < 0 ? 88 : url.Port, cancellationToken);
            var stream = client.GetStream();
            await stream.WriteAsync(data, cancellationToken);

            var header = new byte[4];
            await stream.ReadExactlyAsync(header, cancellationToken);
            var length = BinaryPrimitives.ReadUInt32BigEndian(header);
            if (length > MaxKdcReplyLength)
            {
                throw KdcRequestFailed(url, $"the reply length {length} exceeds {MaxKdcReplyLength} bytes");
            }

            var response = new byte[4 + (int)length];
            header.CopyTo(response, 0);
            await stream.ReadExactlyAsync(response.AsMemory(4), cancellationToken);
            return response;
        }
        catch (Exception ex) when (ex is SocketException or IOException)
        {
            throw KdcRequestFailed(url, ex.Message, ex);
        }
    }

    // KDC proxy (MS-KKDCP section 2.1): sspi wraps the request in a DER-encoded KDC_PROXY_MESSAGE, which is POSTed as
    // the body, and unwraps the KDC_PROXY_MESSAGE in the response body. The reply cap is enforced here rather than
    // through the client's buffer size, so a caller's client gets it too.
    internal static async Task<byte[]> SendKdcProxyRequest(HttpClient client, Uri url, byte[] data,
        CancellationToken cancellationToken)
    {
        try
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, url) { Content = new ByteArrayContent(data) };
            request.Content.Headers.ContentType = new MediaTypeHeaderValue("application/kerberos");
            using var response = await client.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, cancellationToken);
            response.EnsureSuccessStatusCode();

            await using var body = await response.Content.ReadAsStreamAsync(cancellationToken);
            using var reply = new MemoryStream();
            var chunk = new byte[16 * 1024];
            int read;
            while ((read = await body.ReadAsync(chunk, cancellationToken)) > 0)
            {
                if (reply.Length + read > MaxKdcReplyLength)
                {
                    throw KdcRequestFailed(url, $"the reply exceeds {MaxKdcReplyLength} bytes");
                }

                reply.Write(chunk, 0, read);
            }

            return reply.ToArray();
        }
        catch (Exception ex) when (ex is HttpRequestException or IOException)
        {
            throw KdcRequestFailed(url, ex.Message, ex);
        }
    }

    private static IronRdpLibException KdcRequestFailed(Uri url, string reason, Exception? inner = null)
    {
        var message = $"The KDC request to {url} failed: {reason}";
        return inner is null
            ? new IronRdpLibException(IronRdpLibExceptionType.KdcRequestFailed, message)
            : new IronRdpLibException(IronRdpLibExceptionType.KdcRequestFailed, message, inner);
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
