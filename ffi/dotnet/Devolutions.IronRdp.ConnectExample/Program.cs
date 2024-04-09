using System.Drawing;
using System.Drawing.Imaging;

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
                var (res, framed) = await Connection.Connect(buildConfig(serverName, username, password, domain, 1980, 1080), serverName);
                var decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(), res.GetDesktopSize().GetHeight());
                var activeState = ActiveStage.New(res);
                var keepLooping = true;
                while (keepLooping)
                {
                    var readPduTask = framed.ReadPdu();
                    Action? action = null;
                    byte[]? payload = null;
                    if (readPduTask == await Task.WhenAny(readPduTask, Task.Delay(1000)))
                    {
                        var pduReadTask = await readPduTask;
                        action = pduReadTask.Item1;
                        payload = pduReadTask.Item2;
                        Console.WriteLine($"Action: {action}");
                    }
                    else
                    {
                        Console.WriteLine("Timeout");
                        break;
                    }
                    var outputIterator = activeState.Process(decodedImage, action, payload);

                    while (!outputIterator.IsEmpty())
                    {
                        var output = outputIterator.Next()!; // outputIterator.Next() is not null since outputIterator.IsEmpty() is false
                        Console.WriteLine($"Output type: {output.GetType()}");
                        if (output.GetType() == ActiveStageOutputType.Terminate)
                        {
                            Console.WriteLine("Connection terminated.");
                            keepLooping = false;
                        }

                        if (output.GetType() == ActiveStageOutputType.ResponseFrame)
                        {
                            var responseFrame = output.GetResponseFrame()!;
                            byte[] responseFrameBytes = new byte[responseFrame.GetSize()];
                            responseFrame.Fill(responseFrameBytes);
                            await framed.Write(responseFrameBytes);
                        }
                    }
                }

                saveImage(decodedImage, "output.png");

            }
            catch (Exception e)
            {
                Console.WriteLine($"An error occurred: {e.Message}");
            }
        }

        private static void saveImage(DecodedImage decodedImage, string v)
        {
            int width = decodedImage.GetWidth();
            int height = decodedImage.GetHeight();
            var data = decodedImage.GetData();

            var bytes = new byte[data.GetSize()];
            data.Fill(bytes);
            for (int i = 0; i < bytes.Length; i += 4)
            {
                byte temp = bytes[i]; // Store the original Blue value
                bytes[i] = bytes[i + 2]; // Move Red to Blue's position
                bytes[i + 2] = temp; // Move original Blue to Red's position
                                     // Green (bytes[i+1]) and Alpha (bytes[i+3]) remain unchanged
            }
#if WINDOWS // Bitmap is only available on Windows
            using (var bmp = new Bitmap(width, height))
            {
                // Lock the bits of the bitmap.
                var bmpData = bmp.LockBits(new Rectangle(0, 0, bmp.Width, bmp.Height),
                    ImageLockMode.WriteOnly, System.Drawing.Imaging.PixelFormat.Format32bppArgb);

                // Get the address of the first line.
                IntPtr ptr = bmpData.Scan0;
                // Copy the RGBA values back to the bitmap
                System.Runtime.InteropServices.Marshal.Copy(bytes, 0, ptr, bytes.Length);
                // Unlock the bits.
                bmp.UnlockBits(bmpData);

                // Save the bitmap to the specified output path
                bmp.Save("./output.bmp", ImageFormat.Bmp);
            }
#endif

        }

        static Dictionary<string, string>? ParseArguments(string[] args)
        {
            if (args.Length == 0 || Array.Exists(args, arg => arg == "--help"))
            {
                PrintHelp();
                return null;
            }

            var arguments = new Dictionary<string, string>();
            string? lastKey = null;
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



        private static Config buildConfig(string servername, string username, string password, string domain, int width, int height)
        {
            ConfigBuilder configBuilder = ConfigBuilder.New();

            configBuilder.WithUsernameAndPassword(username, password);
            configBuilder.SetDomain(domain);
            configBuilder.SetDesktopSize((ushort)height, (ushort)width);
            configBuilder.SetClientName("IronRdp");
            configBuilder.SetClientDir("C:\\");
            configBuilder.SetPerformanceFlags(PerformanceFlags.NewDefault());

            return configBuilder.Build();
        }

    }
}