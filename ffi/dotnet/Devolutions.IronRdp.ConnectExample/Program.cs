using SixLabors.ImageSharp;
using SixLabors.ImageSharp.PixelFormats;

namespace Devolutions.IronRdp.ConnectExample
{
    class Program
    {
        static async Task Main(string[] args)
        {
            Log.InitLogWithEnv();

            var username = Environment.GetEnvironmentVariable("IRONRDP_USERNAME");
            var password = Environment.GetEnvironmentVariable("IRONRDP_PASSWORD");
            var domain = Environment.GetEnvironmentVariable("IRONRDP_DOMAIN");
            var serverName = Environment.GetEnvironmentVariable("IRONRDP_SERVER");

            if (string.IsNullOrEmpty(username) || string.IsNullOrEmpty(password) || string.IsNullOrEmpty(domain) || string.IsNullOrEmpty(serverName))
            {
                Console.WriteLine("Please set the following environment variables: IRONRDP_USERNAME, IRONRDP_PASSWORD, IRONRDP_DOMAIN, IRONRDP_SERVER");
                return;
            }

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
                        if (output.GetEnumType() == ActiveStageOutputType.Terminate)
                        {
                            Console.WriteLine("Connection terminated.");
                            keepLooping = false;
                        }

                        if (output.GetEnumType() == ActiveStageOutputType.ResponseFrame)
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
                Console.WriteLine($"An error occurred: {e.Message}\n\nStackTrace:\n{e.StackTrace}");
            }
        }

        private static void saveImage(DecodedImage decodedImage, string v)
        {
            int width = decodedImage.GetWidth();
            int height = decodedImage.GetHeight();
            var data = decodedImage.GetData();

            var bytes = new byte[data.GetSize()];
            data.Fill(bytes);

            using Image<Rgba32> image = new Image<Rgba32>(width, height);

            // We’ll mutate this struct instead of creating a new one for performance reasons.
            Rgba32 color = new Rgba32(0, 0, 0);

            for (int col = 0; col < width; ++col)
            {
                for (int row = 0; row < height; ++row)
                {
                    var idx = (row * width + col) * 4;

                    color.R = bytes[idx];
                    color.G = bytes[idx + 1];
                    color.B = bytes[idx + 2];

                    image[col, row] = color;
                }
            }

            // Save the image as bitmap.
            image.Save("./output.bmp");
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
