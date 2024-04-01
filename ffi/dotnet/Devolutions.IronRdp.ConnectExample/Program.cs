using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;
namespace Devolutions.IronRdp.ConnectExample
{
    class Program
    {
        static async Task Main(string[] args)
        {
            try
            {
                var serverName = "IT-HELP-DC.ad.it-help.ninja";
                var username = "Administrator";
                var password = "DevoLabs123!";
                var domain = "ad.it-help.ninja";

                await Connect(serverName, username, password, domain);
            }
            catch (IronRdpException e)
            {
                var err = e.Inner.ToDisplay();
                Console.WriteLine(err);
            }
        }

        static async Task Connect(String servername, String username, String password, String domain)
        {
            SocketAddr serverAddr = SocketAddr.LookUp(servername, 3389);

            ConfigBuilder configBuilder = ConfigBuilder.New();

            configBuilder.WithUsernameAndPasswrord(username, password);
            configBuilder.SetDomain(domain);
            configBuilder.SetDesktopSize(800, 600);
            configBuilder.SetClientName("IronRdp");
            configBuilder.SetClientDir("C:\\");

            Config config = configBuilder.Build();

            ClientConnector connector = ClientConnector.New(config);
            connector.WithServerAddr(serverAddr);

            var writeBuf = WriteBuf.New();
            var stream = await CreateTcpConnection(servername, 3389);
            Console.WriteLine("Connected to server");
            var framed = new Framed<NetworkStream>(stream);
            while (!connector.ShouldPerformSecurityUpgrade())
            {
                await SingleConnectStep(connector, writeBuf, framed);
            }


            Console.WriteLine("need to perform security upgrade");
            var (streamRequireUpgrade, _) = framed.GetInner();
            byte[] serverpubkey = new byte[0];

            var promise = new TaskCompletionSource<bool>();
            var sslStream = new SslStream(streamRequireUpgrade, false, (sender, certificate, chain, sslPolicyErrors) =>
            {
                serverpubkey = certificate!.GetPublicKey();
                promise.SetResult(true);
                return true;
            });
            await sslStream.AuthenticateAsClientAsync(servername);
            await promise.Task;

            var framedSsl = new Framed<SslStream>(sslStream);
            connector.MarkSecurityUpgradeAsDone();
            Console.WriteLine("Security upgrade done");
            if (connector.ShouldPerformCredssp())
            {
                Console.WriteLine("Performing CredSSP");
                await PerformCredsspSteps(connector, ServerName.New(servername), writeBuf, framedSsl, serverpubkey);
            }

            Console.WriteLine("Performing RDP");
            while (!connector.State().IsTerminal())
            {
                await SingleConnectStep(connector, writeBuf, framedSsl);
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
                Console.WriteLine("Resolving generator");
                var clientState = await ResolveGenerator(generator,tcpClient);
                Console.WriteLine("Generator resolved");
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
                if (pduHint != null)
                {
                    break;
                }

                var pdu = await framedSsl.ReadByHint(pduHint!);
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
            Console.WriteLine("Starting generator");
            NetworkStream stream = null;
            while (true)
            {
                if (state.IsSuspended())
                {
                    Console.WriteLine("Generator is suspended");
                    var request = state.GetNetworkRequestIfSuspended()!;
                    var protocol = request.GetProtocol();
                    var url = request.GetUrl();
                    var data = request.GetData();
                    Console.WriteLine("Sending request to " + url);
                    if (null == stream)
                    {
                        url = url.Replace("tcp://", "");
                        var split = url.Split(":");
                        Console.WriteLine("Connecting to " + split[0] + " on port " + split[1]);
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
                    Console.WriteLine("Generator is done");
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

            if (written.IsNothing())
            {
                return;
            }

            var size = written.GetSize();

            if (!size.IsSome())
            {
                Console.WriteLine("Size is nothing");
                return;
            }
            var actualSize = size.Get();

            var response = new byte[actualSize];
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


