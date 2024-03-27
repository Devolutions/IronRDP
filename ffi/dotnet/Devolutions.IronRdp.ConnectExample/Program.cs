using System;
using System.IO.Compression;
using System.Net;
using System.Net.Sockets;
using Devolutions.IronRdp;
namespace Devolutions.IronRdp.ConnectExample
{
    class Program
    {
        static void Main(string[] args)
        {
            try
            {
                var serverName = "IT-HELP-DC.ad.it-help.ninja";
                var username = "Administrator";
                var password = "DevoLabs123!";
                var domain = "ad.it-help.ninja";

                Connect(serverName, username, password, domain);
            }
            catch (IronRdpException e)
            {
                var err = e.Inner.ToDisplay();
                Console.WriteLine(err);
            }
        }

        static async void Connect(String servername, String username, String password, String domain)
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

            while (!connector.ShouldPerformSecurityUpgrade())
            {
                SingleConnectStep(connector, writeBuf,stream);
            }
        }

        static void SingleConnectStep(ClientConnector connector, WriteBuf writeBuf, NetworkStream stream)
        {
            var pduHint = connector.NextPduHint();

            if (pduHint.IsSome()) {
                var pdu = ReadByHints(stream, pduHint);
                connector.Step(pdu, writeBuf);
            } else {
                // connector.Setp
            }


        }
        static async Task<NetworkStream> CreateTcpConnection(String servername, int port)
        {
            IPHostEntry ipHostInfo = await Dns.GetHostEntryAsync(servername);
            IPAddress ipAddress = ipHostInfo.AddressList[0];
            IPEndPoint ipEndPoint = new(ipAddress, port);

            using TcpClient client = new();

            await client.ConnectAsync(ipEndPoint);
            using NetworkStream stream = client.GetStream();

            return stream;
        }

        static VecU8 ReadByHints(NetworkStream stream ,PduHint pduHint) {
            // TODO: Implement ReadByHints
            return VecU8.NewEmpty();
        }
    }
}
