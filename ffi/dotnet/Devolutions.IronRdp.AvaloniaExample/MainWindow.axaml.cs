using Avalonia;
using Avalonia.Controls;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using System;
using System.Net.Security;
using System.Threading.Tasks;

namespace Devolutions.IronRdp.AvaloniaExample;

public partial class MainWindow : Window
{

    WriteableBitmap bitmap;
    Image image;
    public MainWindow()
    {
        InitializeComponent();
        this.Opened += OnOpened;

    }

    private void OnOpened(object sender, EventArgs e)
    {
        Console.WriteLine("OnOpened");

        var username = "Administrator";
        var password = "DevoLabs123!";
        var domain = "ad.it-help.ninja";
        var server = "IT-HELP-DC.ad.it-help.ninja";
        var width = 1280;
        var height = 980;

        var config = buildConfig(server, username, password, domain, width, height);

        var task = Connection.Connect(config, server);
        this.bitmap = new WriteableBitmap(new PixelSize(width, height), new Vector(96, 96), Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque);
        var canvas = this.FindControl<Canvas>("MainCanvas")!;
        this.image = new Image { Width = width, Height = height, Source = this.bitmap };
        canvas.Children.Add(image);

        task.ContinueWith(async t =>
        {
            if (t.IsFaulted)
            {
                return;
            }
            var (res, stream) = t.Result;
            var decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(), res.GetDesktopSize().GetHeight());
            var activeState = ActiveStage.New(res);

            await ProcessActiveState(activeState, decodedImage, stream);
        });
    }

    private void WriteDecodedImageToCanvas(DecodedImage decodedImage)
    {
        Dispatcher.UIThread.InvokeAsync(() =>
        {
            var data = decodedImage.GetData();
            var buffer_size = (int)data.GetSize();
            var buffer = new byte[buffer_size];
            data.Fill(buffer);

            using (var bitmap = this.bitmap.Lock())
            {
                unsafe
                {
                    fixed (byte* p = buffer)
                    {
                        var src = (uint*)p;
                        var dst = (uint*)bitmap.Address;
                        for (var i = 0; i < buffer_size / 4; i++)
                        {
                            dst[i] = src[i];
                        }
                    }
                }
            }

            // Assuming `image` is the Image control that needs to be updated.
            image.InvalidateVisual(); // Force redraw of image
        });
    }


    private async Task ProcessActiveState(ActiveStage activeState, DecodedImage decodedImage, Framed<SslStream> framed)
    {
        var keepLooping = true;
        while (keepLooping)
        {
            var readPduTask = framed.ReadPdu();
            Action? action;
            byte[]? payload;
            if (readPduTask == await Task.WhenAny(readPduTask, Task.Delay(10000)))
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
                    // render the decoded image to canvas
                    WriteDecodedImageToCanvas(decodedImage);
                    // Send the response frame to the server
                    var responseFrame = output.GetResponseFrame()!;
                    byte[] responseFrameBytes = new byte[responseFrame.GetSize()];
                    responseFrame.Fill(responseFrameBytes);
                    await framed.Write(responseFrameBytes);
                }
            }
        }
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
