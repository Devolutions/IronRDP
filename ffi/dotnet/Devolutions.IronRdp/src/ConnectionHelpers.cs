using System.Net.Sockets;

namespace Devolutions.IronRdp;

/// <summary>
/// Internal helper class providing shared connection logic for both direct and RDCleanPath connections.
/// </summary>
internal static class ConnectionHelpers
{
    /// <summary>
    /// Sets up common connector configuration including dynamic channels and clipboard.
    /// </summary>
    internal static void SetupConnector(ClientConnector connector, Config config, CliprdrBackendFactory? factory)
    {
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
    }

    /// <summary>
    /// Performs CredSSP authentication steps over any stream type.
    /// </summary>
    internal static async Task PerformCredsspSteps<T>(
        ClientConnector connector,
        string serverName,
        WriteBuf writeBuf,
        Framed<T> framed,
        byte[] serverpubkey) where T : Stream
    {
        // Extract hostname from "hostname:port" format if needed
        // CredSSP needs just the hostname for the service principal name (TERMSRV/hostname)
        var hostname = serverName;
        var colonIndex = serverName.IndexOf(':');
        if (colonIndex > 0)
        {
            hostname = serverName.Substring(0, colonIndex);
        }

        var credsspSequenceInitResult = CredsspSequence.Init(connector, hostname, serverpubkey, null);
        var credsspSequence = credsspSequenceInitResult.GetCredsspSequence();
        var tsRequest = credsspSequenceInitResult.GetTsRequest();
        var tcpClient = new TcpClient();

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
                await framed.Write(response);
            }

            var pduHint = credsspSequence.NextPduHint();
            if (pduHint == null)
            {
                break;
            }

            var pdu = await framed.ReadByHint(pduHint);
            var decoded = credsspSequence.DecodeServerMessage(pdu);

            // Don't remove, DecodeServerMessage is generated, and it can return null
            if (null == decoded)
            {
                break;
            }

            tsRequest = decoded;
        }
    }

    /// <summary>
    /// Finalizes the RDP connection after security upgrade, performing CredSSP if needed
    /// and completing the connection sequence.
    /// </summary>
    internal static async Task<ConnectionResult> ConnectFinalize<T>(
        string serverName,
        ClientConnector connector,
        byte[] serverPubKey,
        Framed<T> framedSsl) where T : Stream
    {
        var writeBuf = WriteBuf.New();

        if (connector.ShouldPerformCredssp())
        {
            await PerformCredsspSteps(connector, serverName, writeBuf, framedSsl, serverPubKey);
        }

        while (!connector.GetDynState().IsTerminal())
        {
            await Connection.SingleSequenceStep(connector, writeBuf, framedSsl);
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
}
