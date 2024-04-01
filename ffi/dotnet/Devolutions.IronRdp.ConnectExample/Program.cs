using System.Net;
using System.Net.Security;
using System.Net.Sockets;
namespace Devolutions.IronRdp.ConnectExample
{
    class Program
    {
        static async Task Main(string[] args)
        {
            var arguments = ParseArguments(args);

            if (arguments == null) 
            {
                return; 
            }

            var serverName = arguments["--serverName"];
            var username = arguments["--username"];
            var password = arguments["--password"];
            var domain = arguments["--domain"];
            try
            {
                await Connect(serverName, username, password, domain);
            }
            catch (Exception e) 
            {
                Console.WriteLine($"An error occurred: {e.Message}");
            }
        }

        static Dictionary<string, string> ParseArguments(string[] args)
        {
            if (args.Length == 0 || Array.Exists(args, arg => arg == "--help"))
            {
                PrintHelp();
                return null;
            }

            var arguments = new Dictionary<string, string>();
            string lastKey = null;
            foreach (var arg in args)
            {
                if (arg.StartsWith("--"))
                {
                    if (lastKey != null)
                    {
                        Console.WriteLine($"Error: Missing value for {lastKey}.");
                        PrintHelp();
                        return null;
                    }
                    if (!IsValidArgument(arg))
                    {
                        Console.WriteLine($"Error: Unknown argument {arg}.");
                        PrintHelp();
                        return null;
                    }
                    lastKey = arg;
                }
                else
                {
                    if (lastKey == null)
                    {
                        Console.WriteLine("Error: Value without a preceding flag.");
                        PrintHelp();
                        return null;
                    }
                    arguments[lastKey] = arg;
                    lastKey = null;
                }
            }

            if (lastKey != null)
            {
                Console.WriteLine($"Error: Missing value for {lastKey}.");
                PrintHelp();
                return null;
            }

            return arguments;
        }

        static bool IsValidArgument(string argument)
        {
            var validArguments = new List<string> { "--serverName", "--username", "--password", "--domain" };
            return validArguments.Contains(argument);
        }

        static void PrintHelp()
        {
            Console.WriteLine("Usage: dotnet run -- [OPTIONS]");
            Console.WriteLine("Options:");
            Console.WriteLine("  --serverName <serverName>  The name of the server to connect to.");
            Console.WriteLine("  --username <username>      The username for connection.");
            Console.WriteLine("  --password <password>      The password for connection.");
            Console.WriteLine("  --domain <domain>          The domain of the server.");
            Console.WriteLine("  --help                     Show this message and exit.");
        }

        static async Task Connect(String servername, String username, String password, String domain)
        {
            SocketAddr serverAddr;
            Config config = buildConfig(servername, username, password, domain, out serverAddr);

            var stream = await CreateTcpConnection(servername, 3389);
            var framed = new Framed<NetworkStream>(stream);

            ClientConnector connector = ClientConnector.New(config);
            connector.WithServerAddr(serverAddr);

            await connect_begin(framed, connector);
            var (serverPublicKey, framedSsl) = await securityUpgrade(servername, framed, connector);
            await ConnectFinalize(servername, connector, serverPublicKey, framedSsl);
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

        private static async Task connect_begin(Framed<NetworkStream> framed, ClientConnector connector)
        {
            var writeBuf = WriteBuf.New();
            while (!connector.ShouldPerformSecurityUpgrade())
            {
                await SingleConnectStep(connector, writeBuf, framed);
            }
        }

        private static Config buildConfig(string servername, string username, string password, string domain, out SocketAddr serverAddr)
        {
            serverAddr = SocketAddr.LookUp(servername, 3389);
            ConfigBuilder configBuilder = ConfigBuilder.New();

            configBuilder.WithUsernameAndPasswrord(username, password);
            configBuilder.SetDomain(domain);
            configBuilder.SetDesktopSize(800, 600);
            configBuilder.SetClientName("IronRdp");
            configBuilder.SetClientDir("C:\\");

            return configBuilder.Build();
        }

        private static async Task ConnectFinalize(string servername, ClientConnector connector, byte[] serverpubkey, Framed<SslStream> framedSsl)
        {
            var writeBuf2 = WriteBuf.New();
            if (connector.ShouldPerformCredssp())
            {
                await PerformCredsspSteps(connector, ServerName.New(servername), writeBuf2, framedSsl, serverpubkey);
            }
            while (!connector.State().IsTerminal())
            {
                await SingleConnectStep(connector, writeBuf2, framedSsl);
            }
        }

        private static async Task PerformCredsspSteps(ClientConnector connector, ServerName serverName, WriteBuf writeBuf, Framed<SslStream> framedSsl, byte[] serverpubkey)
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
            NetworkStream stream = null;
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
            IPHostEntry ipHostInfo = await Dns.GetHostEntryAsync(servername);
            IPAddress ipAddress = ipHostInfo.AddressList[0];
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
}


