using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace Devolutions.IronRdp;

public enum VmConnectMode
{
    Enhanced,
    Basic,
}

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
    /// <param name="pcb">Optional legacy complete preconnection blob represented as a string</param>
    /// <param name="factory">Optional clipboard backend factory</param>
    /// <returns>A tuple containing the connection result and framed WebSocket stream</returns>
    public static Task<(ConnectionResult, Framed<WebSocketStream>)> ConnectRDCleanPath(
        Config config,
        string gatewayUrl,
        string authToken,
        string destination,
        string? pcb = null,
        CliprdrBackendFactory? factory = null)
    {
        return ConnectRDCleanPathCore(config, gatewayUrl, authToken, destination, pcb, null, factory);
    }

    /// <summary>
    /// Connects to a Hyper-V VM through an RDCleanPath-compatible gateway.
    /// </summary>
    public static Task<(ConnectionResult, Framed<WebSocketStream>)> ConnectVmConnectRDCleanPath(
        Config config,
        string gatewayUrl,
        string authToken,
        string destination,
        string vmId,
        VmConnectMode mode = VmConnectMode.Enhanced,
        CliprdrBackendFactory? factory = null)
    {
        if (string.IsNullOrWhiteSpace(vmId))
        {
            throw new ArgumentException("VMConnect requires a VM ID", nameof(vmId));
        }

        var pcbPayload = mode == VmConnectMode.Enhanced ? $"{vmId};EnhancedMode=1" : vmId;
        return ConnectRDCleanPathCore(config, gatewayUrl, authToken, destination, null, pcbPayload, factory);
    }

    private static async Task<(ConnectionResult, Framed<WebSocketStream>)> ConnectRDCleanPathCore(
        Config config,
        string gatewayUrl,
        string authToken,
        string destination,
        string? pcb,
        string? vmconnectPayload,
        CliprdrBackendFactory? factory)
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
        var (serverPublicKey, framedAfterHandshake, hasX224) = await ConnectRdCleanPath(
            framed, connector, destination, authToken, pcb ?? "", vmconnectPayload);

        if (hasX224)
        {
            // Ordinary front: proxy already performed X.224 and TLS.
            connector.MarkSecurityUpgradeAsDone();
        }
        else
        {
            // PCB front: proxy did PCB + TLS. Client runs CredSSP then HYBRID-only X.224.
            const uint HybridProtocol = 0x00000002;
            var writeBuf = WriteBuf.New();

            await ConnectionHelpers.PerformCredsspSteps(
                connector,
                destination,
                writeBuf,
                framedAfterHandshake,
                serverPublicKey,
                HybridProtocol);

            connector.ClearCredentialsAfterHostAuth();

            writeBuf.Clear();
            var written = connector.InitiateWithSecurityProtocol(HybridProtocol, writeBuf);
            if (written.GetWrittenType() != WrittenType.Nothing)
            {
                var size = (int)written.GetSize().Get();
                var x224Request = new byte[size];
                writeBuf.ReadIntoBuf(x224Request);
                await framedAfterHandshake.Write(x224Request);
            }

            while (!connector.ShouldPerformSecurityUpgrade())
            {
                await Connection.SingleSequenceStep(connector, writeBuf, framedAfterHandshake);
            }

            connector.EnsureSelectedHybrid();
            connector.MarkSecurityUpgradeAsDone();
            connector.MarkCredsspAsDone();
        }

        // Step 6: Finalize connection
        System.Diagnostics.Debug.WriteLine("Finalizing RDP connection...");
        var result = await ConnectionHelpers.ConnectFinalize(destination, connector, serverPublicKey, framedAfterHandshake);

        System.Diagnostics.Debug.WriteLine("Gateway connection established successfully!");
        return (result, framedAfterHandshake);
    }

    /// <summary>
    /// Performs the RDCleanPath handshake with the RDCleanPath-compatible gateway.
    /// </summary>
    private static async Task<(byte[], Framed<WebSocketStream>, bool)> ConnectRdCleanPath(
        Framed<WebSocketStream> framed,
        ClientConnector connector,
        string destination,
        string authToken,
        string pcb,
        string? vmconnectPayload)
    {
        var writeBuf = WriteBuf.New();
        var vmconnect = vmconnectPayload != null;

        System.Diagnostics.Debug.WriteLine($"Sending RDCleanPath request to {destination}...");
        RDCleanPathPdu rdCleanPathReq;
        if (vmconnect)
        {
            rdCleanPathReq = RDCleanPathPdu.NewVmconnectRequest(destination, authToken, vmconnectPayload!);
        }
        else
        {
            var written = connector.StepNoInput(writeBuf);
            var firstPduSize = (int)written.GetSize().Get();
            var firstPdu = new byte[firstPduSize];
            writeBuf.ReadIntoBuf(firstPdu);
            rdCleanPathReq = RDCleanPathPdu.NewRequest(firstPdu, destination, authToken, pcb);
        }
        var reqBytes = rdCleanPathReq.ToDer();
        var reqBytesArray = new byte[reqBytes.GetSize()];
        reqBytes.Fill(reqBytesArray);
        await framed.Write(reqBytesArray);

        System.Diagnostics.Debug.WriteLine("Waiting for RDCleanPath response...");
        var respBytes = await framed.ReadByHint(new RDCleanPathHint());
        var rdCleanPathResp = RDCleanPathPdu.FromDer(respBytes);

        var resultType = rdCleanPathResp.GetType();

        if (resultType == RDCleanPathResultType.Response)
        {
            System.Diagnostics.Debug.WriteLine("RDCleanPath handshake successful!");

            var hasX224 = rdCleanPathResp.HasX224();
            if (vmconnect == hasX224)
            {
                throw new IronRdpLibException(
                    IronRdpLibExceptionType.ConnectionFailed,
                    vmconnect
                        ? "RDCleanPath response includes X.224 for a VMConnect request"
                        : "RDCleanPath response missing X.224 for an ordinary request");
            }

            if (hasX224)
            {
                var x224Response = rdCleanPathResp.GetX224Response();
                var x224ResponseBytes = new byte[x224Response.GetSize()];
                x224Response.Fill(x224ResponseBytes);

                writeBuf.Clear();
                connector.Step(x224ResponseBytes, writeBuf);
            }

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

            return (serverPublicKey, framed, hasX224);
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
