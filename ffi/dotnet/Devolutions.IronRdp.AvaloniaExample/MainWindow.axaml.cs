using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using System;
using System.Net.Security;
using System.Threading.Tasks;

namespace Devolutions.IronRdp.AvaloniaExample;

public partial class MainWindow : Window
{

    WriteableBitmap? bitmap;
    Canvas? canvas;
    Image? image;
    InputDatabase? inputDatabase = InputDatabase.New();
    ActiveStage? activeStage;
    DecodedImage? decodedImage;
    Framed<SslStream>? framed;
    public MainWindow()
    {
        InitializeComponent();
        this.Opened += OnOpened;

    }

    private void OnOpened(object? sender, EventArgs e)
    {
        Console.WriteLine("OnOpened");

        var username = "Administrator";
        var password = "DevoLabs123!";
        var domain = "ad.it-help.ninja";
        var server = "IT-HELP-DC.ad.it-help.ninja";
        var width = 1280;
        var height = 800;

        var config = buildConfig(username, password, domain, width, height);

        var task = Connection.Connect(config, server);
        bitmap = new WriteableBitmap(new PixelSize(width, height), new Vector(96, 96), Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque);
        canvas = this.FindControl<Canvas>("MainCanvas")!;
        canvas.Focusable = true;
        image = new Image { Width = width, Height = height, Source = this.bitmap };
        canvas.Children.Add(image);

        canvas.KeyDown += Canvas_KeyDown;
        canvas.KeyUp += Canvas_KeyUp;

        task.ContinueWith(t =>
        {
            if (t.IsFaulted)
            {
                return;
            }
            var (res, framed) = t.Result;
            this.decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(), res.GetDesktopSize().GetHeight());
            this.activeStage = ActiveStage.New(res);
            this.framed = framed;
            ReadPduAndProcessActiveStage();
        });
    }

    private void WriteDecodedImageToCanvas()
    {
        Dispatcher.UIThread.InvokeAsync(() =>
        {
            var data = decodedImage!.GetData();
            var buffer_size = (int)data.GetSize();
            var buffer = new byte[buffer_size];
            data.Fill(buffer);

            using (var bitmap = this.bitmap!.Lock())
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
            image!.InvalidateVisual(); // Force redraw of image
        });
    }


    private void ReadPduAndProcessActiveStage()
    {
        Task.Run(async () =>
        {
            var keepLooping = true;
            while (keepLooping)
            {
                Console.WriteLine("Reading PDU , updateCounter = " + updateCounter);
                var readPduTask = await framed!.ReadPdu();
                Action action = readPduTask.Item1;
                byte[] payload = readPduTask.Item2;
                var outputIterator = activeStage!.Process(decodedImage!, action, payload);
                keepLooping = await HandleActiveStageOutput(outputIterator);
            }
        });
    }

    private static Config buildConfig(string username, string password, string domain, int width, int height)
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

    private void Canvas_OnPointerPressed(object sender, Avalonia.Input.PointerPressedEventArgs e)
    {
        Console.WriteLine("Mouse pressed");
        PointerUpdateKind mouseButton = e.GetCurrentPoint((Visual?)sender).Properties.PointerUpdateKind;

        MouseButtonType buttonType = mouseButton switch
        {
            PointerUpdateKind.LeftButtonPressed => MouseButtonType.Left,
            PointerUpdateKind.RightButtonPressed => MouseButtonType.Right,
            PointerUpdateKind.MiddleButtonPressed => MouseButtonType.Middle,
            PointerUpdateKind.XButton1Pressed => MouseButtonType.X1,
            PointerUpdateKind.XButton2Pressed => MouseButtonType.X2,
            PointerUpdateKind.LeftButtonReleased => MouseButtonType.Left,
            PointerUpdateKind.MiddleButtonReleased => MouseButtonType.Middle,
            PointerUpdateKind.RightButtonReleased => MouseButtonType.Right,
            PointerUpdateKind.XButton1Released => MouseButtonType.X1,
            PointerUpdateKind.XButton2Released => MouseButtonType.X2,
            PointerUpdateKind.Other => throw new NotImplementedException(),
            _ => throw new NotImplementedException(),
        };

        var buttonOperation = MouseButton.New(buttonType).AsOperationMouseButtonPressed();
        var fastpath = inputDatabase!.Apply(buttonOperation);
        var output = activeStage!.ProcessFastpathInput(decodedImage!, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_PointerMoved(object sender, PointerEventArgs e)
    {
        if (this.activeStage == null || this.decodedImage == null)
        {
            return;
        }
        var position = e.GetPosition((Visual?)sender);
        var x = (ushort)position.X;
        var y = (ushort)position.Y;
        var mouseMovedEvent = MousePosition.New(x, y).AsOperation();
        var fastpath = inputDatabase!.Apply(mouseMovedEvent);
        var output = activeStage.ProcessFastpathInput(decodedImage, fastpath);
        Console.WriteLine($"Pointer moved to X: {x}, Y: {y}");
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_PointerReleased(object sender, PointerReleasedEventArgs e)
    {
        Console.WriteLine("Mouse released");
        PointerUpdateKind mouseButton = e.GetCurrentPoint((Visual?)sender).Properties.PointerUpdateKind;

        MouseButtonType buttonType = mouseButton switch
        {
            PointerUpdateKind.LeftButtonPressed => MouseButtonType.Left,
            PointerUpdateKind.RightButtonPressed => MouseButtonType.Right,
            PointerUpdateKind.MiddleButtonPressed => MouseButtonType.Middle,
            PointerUpdateKind.XButton1Pressed => MouseButtonType.X1,
            PointerUpdateKind.XButton2Pressed => MouseButtonType.X2,
            PointerUpdateKind.LeftButtonReleased => MouseButtonType.Left,
            PointerUpdateKind.MiddleButtonReleased => MouseButtonType.Middle,
            PointerUpdateKind.RightButtonReleased => MouseButtonType.Right,
            PointerUpdateKind.XButton1Released => MouseButtonType.X1,
            PointerUpdateKind.XButton2Released => MouseButtonType.X2,
            PointerUpdateKind.Other => throw new NotImplementedException(),
            _ => throw new NotImplementedException(),
        };

        var buttonOperation = MouseButton.New(buttonType).AsOperationMouseButtonReleased();
        var fastpath = inputDatabase!.Apply(buttonOperation);
        var output = activeStage!.ProcessFastpathInput(decodedImage!, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_KeyDown(object? sender, KeyEventArgs? e)
    {
        if (activeStage == null || decodedImage == null)
        {
            return;
        }
        Console.WriteLine($"Key pressed: {e!.Key}");
        PhysicalKey physicalKey = e.PhysicalKey;
        var keyOperation = Scancode.FromU16(KeyCodeMapper.GetScancode(physicalKey)).AsOperationKeyPressed();
        var fastpath = inputDatabase!.Apply(keyOperation);
        var output = activeStage.ProcessFastpathInput(decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_KeyUp(object? sender, KeyEventArgs? e)
    {
        if (this.activeStage == null || this.decodedImage == null)
        {
            return;
        }
        Console.WriteLine($"Key released: {e!.Key}");
        Key key = e.Key;
        var keyOperation = Scancode.FromU16((ushort)key).AsOperationKeyReleased();
        var fastpath = inputDatabase!.Apply(keyOperation);
        var output = activeStage.ProcessFastpathInput(decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private ulong updateCounter = 0;
    private async Task<bool> HandleActiveStageOutput(ActiveStageOutputIterator outputIterator)
    {
        try
        {

            while (!outputIterator.IsEmpty())
            {
                var output = outputIterator.Next()!; // outputIterator.Next() is not null since outputIterator.IsEmpty() is false
                Console.WriteLine("Handling output = " + updateCounter++, "output type " + output.GetEnumType());
                if (output.GetEnumType() == ActiveStageOutputType.Terminate)
                {
                    return false;
                }
                else if (output.GetEnumType() == ActiveStageOutputType.ResponseFrame)
                {
                    // render the decoded image to canvas
                    WriteDecodedImageToCanvas();
                    // Send the response frame to the server
                    var responseFrame = output.GetResponseFrame()!;
                    byte[] responseFrameBytes = new byte[responseFrame.GetSize()];
                    responseFrame.Fill(responseFrameBytes);
                    await framed!.Write(responseFrameBytes);
                }
                else if (output.GetEnumType() == ActiveStageOutputType.GraphicsUpdate)
                {
                    WriteDecodedImageToCanvas();
                }
                else if (output.GetEnumType() == ActiveStageOutputType.PointerPosition)
                {
                    WriteDecodedImageToCanvas();
                }
                else if (output.GetEnumType() == ActiveStageOutputType.PointerBitmap)
                {
                    WriteDecodedImageToCanvas();
                }
                else
                {
                    Console.WriteLine("Unhandled output type " + output.GetEnumType());
                }
            }
            return true;
        }
        catch (Exception e)
        {
            Console.WriteLine("Error handling output: " + e.Message);
            return false;
        }
    }

}
