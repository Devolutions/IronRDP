using System;
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

        static void Connect(String servername, String username, String password, String domain)
        {
            SocketAddr serverAddr = SocketAddr.LookUp(servername, 3389);

            StdTcpStream stream = StdTcpStream.Connect(serverAddr);

            BlockingTcpFrame tcpFrame = BlockingTcpFrame.FromTcpStream(stream);

            ConfigBuilder configBuilder = ConfigBuilder.New();

            // Password is wrong
            configBuilder.WithUsernameAndPasswrord(username, password);
            configBuilder.SetDomain(domain);
            configBuilder.SetDesktopSize(800, 600);
            configBuilder.SetClientName("IronRdp");
            configBuilder.SetClientDir("C:\\");

            Config config = configBuilder.Build();

            ClientConnector connector = ClientConnector.New(config);
            connector.WithServerAddr(serverAddr);

            ShouldUpgrade shouldUpgrade = IronRdpBlocking.ConnectBegin(tcpFrame, connector);

            var tcpStream = tcpFrame.IntoTcpSteamNoLeftover();

            var tlsUpgradeResult = Tls.TlsUpgrade(tcpStream, servername);
            var upgraded = IronRdpBlocking.MarkAsUpgraded(shouldUpgrade, connector);

            var upgradedStream = tlsUpgradeResult.GetUpgradedStream();
            var serverPublicKey = tlsUpgradeResult.GetServerPublicKey();

            var upgradedFrame = BlockingUpgradedFrame.FromUpgradedStream(upgradedStream);

            var serverName = ServerName.New(servername);

            var connectorResult = IronRdpBlocking.ConnectFinalize(upgraded, upgradedFrame, connector, serverName, serverPublicKey, null);

        }
    }
}
