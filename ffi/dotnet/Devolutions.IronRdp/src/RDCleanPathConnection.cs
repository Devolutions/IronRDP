using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace Devolutions.IronRdp;

/// <summary>
/// Provides methods for connecting to RDP servers through an RDCleanPath-compatible gateway
/// (such as Devolutions Gateway or Cloudflare) using WebSocket.
/// </summary>
public static class RDCleanPathConnection
{
    /// <summary>
    /// Connects to an RDP server through an RDCleanPath-compatible gateway using WebSocket.
    /// </summary>
    /// <param name="config">The RDP connection configuration</param>
    /// <param name="gatewayUrl">The WebSocket URL to the RDCleanPath gateway (e.g., "ws://localhost:7171/jet/rdp")</param>
    /// <param name="authToken">The JWT authentication token for the RDCleanPath gateway</param>
    /// <param name="destination">The destination RDP server address (e.g., "10.10.0.3:3389")</param>
    /// <param name="pcb">Optional preconnection blob for Hyper-V VM connections</param>
    /// <param name="factory">Optional clipboard backend factory</param>
    /// <returns>A tuple containing the connection result and framed WebSocket stream</returns>
    public static async Task<(ConnectionResult, Framed<WebSocketStream>)> ConnectRDCleanPath(
        Config config,
        string gatewayUrl,
        string authToken,
        string destination,
        string? pcb = null,
        CliprdrBackendFactory? factory = null)
    {
        // Step 1: Connect WebSocket to gateway
        System.Diagnostics.Debug.WriteLine($"Connecting to gateway at {gatewayUrl}...");
        var ws = await WebSocketStream.ConnectAsync(new Uri(gatewayUrl));
        var framed = new Framed<WebSocketStream>(ws);

        // Step 2: Get client local address from the WebSocket connection
        // This mimics Rust: let client_addr = socket.local_addr()?;
        string clientAddr = ws.ClientAddr;
        System.Diagnostics.Debug.WriteLine($"Client local address: {clientAddr}");

        // Step 3: Setup ClientConnector
        var connector = ClientConnector.New(config, clientAddr);
        ConnectionHelpers.SetupConnector(connector, config, factory);

        // Step 4: Perform RDCleanPath handshake
        System.Diagnostics.Debug.WriteLine("Performing RDCleanPath handshake...");
        var (serverPublicKey, framedAfterHandshake) = await ConnectRdCleanPath(
            framed, connector, destination, authToken, pcb ?? "");

        // Step 5: Mark security upgrade as done (WebSocket already has TLS)
        connector.MarkSecurityUpgradeAsDone();

        // Step 6: Finalize connection
        System.Diagnostics.Debug.WriteLine("Finalizing RDP connection...");
        var result = await ConnectionHelpers.ConnectFinalize(destination, connector, serverPublicKey, framedAfterHandshake);

        System.Diagnostics.Debug.WriteLine("Gateway connection established successfully!");
        return (result, framedAfterHandshake);
    }

    /// <summary>
    /// Performs the RDCleanPath handshake with the RDCleanPath-compatible gateway.
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
        System.Diagnostics.Debug.WriteLine("Generating X.224 Connection Request...");
        var written = connector.StepNoInput(writeBuf);
        var x224PduSize = (int)written.GetSize().Get();
        var x224Pdu = new byte[x224PduSize];
        writeBuf.ReadIntoBuf(x224Pdu);

        // Step 2: Create and send RDCleanPath Request
        System.Diagnostics.Debug.WriteLine($"Sending RDCleanPath request to {destination}...");
        var rdCleanPathReq = RDCleanPathPdu.NewRequest(x224Pdu, destination, authToken, pcb);
        var reqBytes = rdCleanPathReq.ToDer();
        var reqBytesArray = new byte[reqBytes.GetSize()];
        reqBytes.Fill(reqBytesArray);
        await framed.Write(reqBytesArray);

        // Step 3: Read RDCleanPath Response
        System.Diagnostics.Debug.WriteLine("Waiting for RDCleanPath response...");
        var respBytes = await framed.ReadByHint(new RDCleanPathHint());
        var rdCleanPathResp = RDCleanPathPdu.FromDer(respBytes);

        // Step 4: Determine response type and handle accordingly
        var resultType = rdCleanPathResp.GetType();

        if (resultType == RDCleanPathResultType.Response)
        {
            System.Diagnostics.Debug.WriteLine("RDCleanPath handshake successful!");

            // Extract X.224 response
            var x224Response = rdCleanPathResp.GetX224Response();
            var x224ResponseBytes = new byte[x224Response.GetSize()];
            x224Response.Fill(x224ResponseBytes);

            // Process X.224 response with connector
            writeBuf.Clear();
            connector.Step(x224ResponseBytes, writeBuf);

            // Extract server public key from certificate chain
            var certChain = rdCleanPathResp.GetServerCertChain();
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

            System.Diagnostics.Debug.WriteLine($"Extracted server public key (length: {serverPublicKey.Length})");

            return (serverPublicKey, framed);
        }
        else if (resultType == RDCleanPathResultType.GeneralError)
        {
            var errorCode = rdCleanPathResp.GetErrorCode();
            var errorMessage = rdCleanPathResp.GetErrorMessage();
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
