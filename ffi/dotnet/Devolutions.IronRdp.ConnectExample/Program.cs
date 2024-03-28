using System.Net;
using System.Net.Sockets;
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
        }

        static async Task SingleConnectStep(ClientConnector connector, WriteBuf buf, Framed<NetworkStream> framed)
        {
            buf.Clear();

            var pduHint = connector.NextPduHint();
            Written written;
            if (pduHint.IsSome())
            {
                byte[] pdu = await framed.ReadByHint(pduHint);
                written = connector.Step(pdu,buf);
            }
            else
            {
                written = connector.StepNoInput(buf);
            }

            if (written.IsNothing())
            {
                Console.WriteLine("Written is nothing");
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
}
