using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace Devolutions.IronRdp;

/// <summary>
/// Provides methods for connecting to RDP servers through Devolutions Gateway
/// using the RDCleanPath protocol over WebSocket.
/// </summary>
public static class GatewayConnection
{
    /// <summary>
    /// Connects to an RDP server through a Devolutions Gateway using WebSocket and RDCleanPath protocol.
    /// </summary>
    /// <param name="config">The RDP connection configuration</param>
    /// <param name="gatewayUrl">The WebSocket URL to the gateway (e.g., "ws://localhost:7171/jet/rdp")</param>
    /// <param name="authToken">The JWT authentication token for the gateway</param>
    /// <param name="destination">The destination RDP server address (e.g., "10.10.0.3:3389")</param>
    /// <param name="pcb">Optional preconnection blob for Hyper-V VM connections</param>
    /// <param name="factory">Optional clipboard backend factory</param>
    /// <param name="kdcProxyUrl">Optional KDC proxy URL with token (e.g., "https://gateway.example.com/KdcProxy/{token}")</param>
    /// <param name="kdcHostname">Optional client hostname for Kerberos</param>
    /// <returns>A tuple containing the connection result and framed WebSocket stream</returns>
    public static async Task<(ConnectionResult, Framed<WebSocketStream>)> ConnectViaGateway(
        Config config,
        string gatewayUrl,
        string authToken,
        string destination,
        string? pcb = null,
        CliprdrBackendFactory? factory = null,
        string? kdcProxyUrl = null,
        string? kdcHostname = null)
    {
        // Step 1: Connect WebSocket to gateway
        Console.WriteLine($"Connecting to gateway at {gatewayUrl}...");
        var ws = await WebSocketStream.ConnectAsync(new Uri(gatewayUrl));
        var framed = new Framed<WebSocketStream>(ws);

        // Step 2: Get client local address (dummy for WebSocket)
        string clientAddr = "127.0.0.1:33899";

        // Step 3: Setup ClientConnector
        var connector = ClientConnector.New(config, clientAddr);

        // Attach optional dynamic/static channels
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

        // Step 4: Perform RDCleanPath handshake
        Console.WriteLine("Performing RDCleanPath handshake...");
        var (serverPublicKey, framedAfterHandshake) = await ConnectRdCleanPath(
            framed, connector, destination, authToken, pcb ?? "");

        // Step 5: Mark security upgrade as done (WebSocket already has TLS)
        connector.MarkSecurityUpgradeAsDone();

        // Step 6: Finalize connection
        Console.WriteLine("Finalizing RDP connection...");
        var result = await ConnectFinalize(destination, connector, serverPublicKey, framedAfterHandshake, kdcProxyUrl, kdcHostname);

        Console.WriteLine("Gateway connection established successfully!");
        return (result, framedAfterHandshake);
    }

    /// <summary>
    /// Performs the RDCleanPath handshake with the gateway.
    /// </summary>
    private static async Task<(byte[], Framed<WebSocketStream>)> ConnectRdCleanPath(
        Framed<WebSocketStream> framed,
        ClientConnector connector,
        string destination,
        string authToken,
        string pcb)
    {
        var writeBuf = WriteBuf.New();

        // Step 1: Generate X.224 Connection Request
        Console.WriteLine("Generating X.224 Connection Request...");
        var written = connector.StepNoInput(writeBuf);
        var x224PduSize = (int)written.GetSize().Get();
        var x224Pdu = new byte[x224PduSize];
        writeBuf.ReadIntoBuf(x224Pdu);

        // Step 2: Create and send RDCleanPath Request
        Console.WriteLine($"Sending RDCleanPath request to {destination}...");
        var rdCleanPathReq = RDCleanPathPdu.NewRequest(x224Pdu, destination, authToken, pcb);
        var reqBytes = rdCleanPathReq.ToDer();
        var reqBytesArray = new byte[reqBytes.GetSize()];
        reqBytes.Fill(reqBytesArray);
        await framed.Write(reqBytesArray);

        // Step 3: Read RDCleanPath Response
        Console.WriteLine("Waiting for RDCleanPath response...");
        var respBytes = await framed.ReadByHint(new RDCleanPathHint());
        var rdCleanPathResp = RDCleanPathPdu.FromDer(respBytes);

        // Step 4: Parse response
        var result = rdCleanPathResp.IntoEnum();
        var resultType = result.GetType();

        if (resultType == RDCleanPathResultType.Response)
        {
            Console.WriteLine("RDCleanPath handshake successful!");

            // Extract X.224 response
            var x224Response = result.GetX224Response();
            var x224ResponseBytes = new byte[x224Response.GetSize()];
            x224Response.Fill(x224ResponseBytes);

            // Process X.224 response with connector
            writeBuf.Clear();
            connector.Step(x224ResponseBytes, writeBuf);

            // Extract server public key from certificate chain
            var certChain = result.GetServerCertChain();
            if (certChain.IsEmpty())
            {
                throw new IronRdpLibException(
                    IronRdpLibExceptionType.ConnectionFailed,
                    "Server certificate chain is empty");
            }

            var firstCert = certChain.Next();
            if (firstCert == null)
            {
                throw new IronRdpLibException(
                    IronRdpLibExceptionType.ConnectionFailed,
                    "Failed to get first certificate from chain");
            }

            var certBytes = new byte[firstCert.GetSize()];
            firstCert.Fill(certBytes);

            var serverPublicKey = ExtractPublicKeyFromX509(certBytes);

            Console.WriteLine($"Extracted server public key (length: {serverPublicKey.Length})");

            return (serverPublicKey, framed);
        }
        else if (resultType == RDCleanPathResultType.GeneralError)
        {
            var errorCode = result.GetErrorCode();
            var errorMessage = result.GetErrorMessage();
            throw new IronRdpLibException(
                IronRdpLibExceptionType.ConnectionFailed,
                $"RDCleanPath error (code {errorCode}): {errorMessage}");
        }
        else if (resultType == RDCleanPathResultType.NegotiationError)
        {
            throw new IronRdpLibException(
                IronRdpLibExceptionType.ConnectionFailed,
                "RDCleanPath negotiation error: Server rejected connection parameters");
        }
        else
        {
            throw new IronRdpLibException(
                IronRdpLibExceptionType.ConnectionFailed,
                $"Unexpected RDCleanPath response type: {resultType}");
        }
    }

    /// <summary>
    /// Finalizes the RDP connection after RDCleanPath handshake.
    /// </summary>
    private static async Task<ConnectionResult> ConnectFinalize(
        string serverName,
        ClientConnector connector,
        byte[] serverPubKey,
        Framed<WebSocketStream> framedSsl,
        string? kdcProxyUrl,
        string? kdcHostname)
    {
        var writeBuf = WriteBuf.New();

        // Perform CredSSP if needed
        if (connector.ShouldPerformCredssp())
        {
            Console.WriteLine("Performing CredSSP authentication...");
            await PerformCredsspSteps(connector, serverName, writeBuf, framedSsl, serverPubKey, kdcProxyUrl, kdcHostname);
        }

        // Continue with remaining connection steps
        Console.WriteLine("Completing connection sequence...");
        while (!connector.GetDynState().IsTerminal())
        {
            await Connection.SingleSequenceStep(connector, writeBuf, framedSsl);
        }

        // Get final connection result
        ClientConnectorState state = connector.ConsumeAndCastToClientConnectorState();

        if (state.GetEnumType() == ClientConnectorStateType.Connected)
        {
            return state.GetConnectedResult();
        }
        else
        {
            throw new IronRdpLibException(
                IronRdpLibExceptionType.ConnectionFailed,
                "Connection failed after RDCleanPath handshake");
        }
    }

    /// <summary>
    /// Performs CredSSP authentication steps.
    /// </summary>
    private static async Task PerformCredsspSteps(
        ClientConnector connector,
        string serverName,
        WriteBuf writeBuf,
        Framed<WebSocketStream> framedSsl,
        byte[] serverpubkey,
        string? kdcProxyUrl,
        string? kdcHostname)
    {
        // Extract hostname from "hostname:port" format for CredSSP
        // CredSSP needs just the hostname for the service principal name (TERMSRV/hostname)
        var hostname = serverName;
        var colonIndex = serverName.IndexOf(':');
        if (colonIndex > 0)
        {
            hostname = serverName.Substring(0, colonIndex);
        }

        // Create KerberosConfig if KDC proxy URL is provided
        KerberosConfig? kerberosConfig = null;
        if (!string.IsNullOrEmpty(kdcProxyUrl))
        {
            Console.WriteLine($"Using KDC proxy: {kdcProxyUrl}");
            kerberosConfig = KerberosConfig.New(kdcProxyUrl, kdcHostname ?? "");
        }

        var credsspSequenceInitResult = CredsspSequence.Init(connector, hostname, serverpubkey, kerberosConfig);
        var credsspSequence = credsspSequenceInitResult.GetCredsspSequence();
        var tsRequest = credsspSequenceInitResult.GetTsRequest();
        var tcpClient = new System.Net.Sockets.TcpClient();

        while (true)
        {
            var generator = credsspSequence.ProcessTsRequest(tsRequest);
            var clientState = await Connection.ResolveGenerator(generator, tcpClient);
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

            if (null == decoded)
            {
                break;
            }

            tsRequest = decoded;
        }
    }

    /// <summary>
    /// Extracts the public key from an X.509 certificate in DER format.
    /// </summary>
    private static byte[] ExtractPublicKeyFromX509(byte[] certDer)
    {
        try
        {
            var cert = new X509Certificate2(certDer);
            return cert.GetPublicKey();
        }
        catch (Exception ex)
        {
            throw new IronRdpLibException(
                IronRdpLibExceptionType.ConnectionFailed,
                $"Failed to extract public key from certificate: {ex.Message}");
        }
    }
}

/// <summary>
/// PDU hint for detecting RDCleanPath PDUs in the stream.
/// </summary>
public class RDCleanPathHint : IPduHint
{
    public (bool, int)? FindSize(byte[] bytes)
    {
        var detection = RDCleanPathPdu.Detect(bytes);

        if (detection.IsDetected())
        {
            var totalLength = (int)detection.GetTotalLength();
            return (true, totalLength);
        }

        if (detection.IsNotEnoughBytes())
        {
            return null; // Need more bytes
        }

        // Detection failed
        throw new IronRdpLibException(
            IronRdpLibExceptionType.ConnectionFailed,
            "Invalid RDCleanPath PDU detected");
    }
}
