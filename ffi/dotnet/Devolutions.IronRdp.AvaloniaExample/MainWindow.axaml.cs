using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using System;
using System.Diagnostics;
using System.Net.Security;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Avalonia.Markup.Xaml;

namespace Devolutions.IronRdp.AvaloniaExample;

public partial class MainWindow : Window
{
    WriteableBitmap? _bitmap;
    Canvas? _canvas;
    Image? _image;
    readonly InputDatabase? _inputDatabase = InputDatabase.New();
    ActiveStage? _activeStage;
    DecodedImage? _decodedImage;
    Framed<SslStream>? _framed;
    WinCliprdr? _cliprdr;

    public MainWindow()
    {
        InitializeComponent();
        this.Opened += OnOpened;
    }

    private void InitializeComponent()
    {
        AvaloniaXamlLoader.Load(this);
    }

    private void OnOpened(object? sender, EventArgs e)
    {
        Log.InitWithEnv();

        WindowState = WindowState.Maximized;

        var username = Environment.GetEnvironmentVariable("IRONRDP_USERNAME");
        var password = Environment.GetEnvironmentVariable("IRONRDP_PASSWORD");
        var domain = Environment.GetEnvironmentVariable("IRONRDP_DOMAIN");
        var server = Environment.GetEnvironmentVariable("IRONRDP_SERVER");

        if (username == null || password == null || domain == null || server == null)
        {
            Trace.TraceError(
                "Please set the IRONRDP_USERNAME, IRONRDP_PASSWORD, IRONRDP_DOMAIN, and IRONRDP_SERVER environment variables");
            Close();
            return;
        }

        const int width = 1280;
        const int height = 800;

        var config = BuildConfig(username, password, domain, width, height);

        CliprdrBackendFactory? factory = null;
        var handle = GetWindowHandle();
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows) && handle != null)
        {
            switch (RuntimeInformation.ProcessArchitecture)
            {
                case Architecture.X64:
                case Architecture.Arm64:
                    _cliprdr = WinCliprdr.New64bit((ulong)handle.Value.ToInt64());
                    break;
                case Architecture.X86:
                case Architecture.Arm:
                    _cliprdr = WinCliprdr.New32bit((uint)handle.Value.ToInt32());
                    break;
            }

            if (_cliprdr != null)
            {
                factory = _cliprdr.BackendFactory();
            }
        }


        var task = Connection.Connect(config, server, factory);

        PostConnectSetup(width, height);

        task.ContinueWith(t =>
        {
            if (t.IsFaulted)
            {
                Exception connectError = t.Exception!;
                Trace.TraceError("Error connecting to server: " + connectError.Message);
                Close();
                return;
            }

            var (res, framed) = t.Result;
            this._decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(),
                res.GetDesktopSize().GetHeight());
            this._activeStage = ActiveStage.New(res);
            this._framed = framed;
            ReadPduAndProcessActiveStage();
            HandleClipboardEvents();
        }).ContinueWith(t =>
        {
            if (t.IsFaulted)
            {
                Trace.TraceError("Error processing active stage: " + t.Exception!.Message);
                Close();
            }
            return;
        });
    }

    private void PostConnectSetup(int width, int height)
    {
        _bitmap = new WriteableBitmap(new PixelSize(width, height), new Vector(96, 96),
            Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque);
        _canvas = this.FindControl<Canvas>("MainCanvas")!;
        _canvas.Focusable = true;
        _image = new Image { Width = width, Height = height, Source = this._bitmap };
        _canvas.Children.Add(_image);
        _canvas.KeyDown += Canvas_KeyDown;
        _canvas.KeyUp += Canvas_KeyUp;
    }

    private async void WriteDecodedImageToCanvas()
    {
        await Dispatcher.UIThread.InvokeAsync(() =>
        {
            var data = _decodedImage!.GetData();
            var bufferSize = (int)data.GetSize();

            var buffer = new byte[bufferSize];
            data.Fill(buffer);

            using (var bitmap = this._bitmap!.Lock())
            {
                unsafe
                {
                    var bitmapSpan = new Span<byte>((void*)bitmap.Address, bufferSize);
                    var bufferSpan = new Span<byte>(buffer);
                    bufferSpan.CopyTo(bitmapSpan);
                }
            }

            _image!.InvalidateVisual();
        });
    }

    private void ReadPduAndProcessActiveStage()
    {
        Task.Run(async () =>
        {
            var keepLooping = true;
            while (keepLooping)
            {
                var readPduTask = await _framed!.ReadPdu();
                Action action = readPduTask.Item1;
                byte[] payload = readPduTask.Item2;
                var outputIterator = _activeStage!.Process(_decodedImage!, action, payload);
                keepLooping = await HandleActiveStageOutput(outputIterator);
            }
        });
    }

    private void HandleClipboardEvents()
    {
        Task.Run(async () =>
        {
            while (true)
            {
                if (_cliprdr == null)
                {
                    continue;
                }

                var message = _cliprdr.NextClipboardMessageBlocking();

                var clipBoard = _activeStage!.GetSvcProcessorCliprdr();
                VecU8 frame;
                var messageType = message.GetMessageType();
                Trace.TraceInformation("Clipboard message type: " + messageType);
                if (messageType == ClipboardMessageType.SendFormatData)
                {
                    var formatData = message.GetSendFormatData()!;
                    var svgMessage = clipBoard.SubmitFormatData(formatData);
                    frame = _activeStage.ProcessSvcProcessorMessageCliprdr(svgMessage);
                }
                else if (messageType == ClipboardMessageType.SendInitiateCopy)
                {
                    var initiateCopy = message.GetSendInitiateCopy()!;
                    var svgMessage = clipBoard.InitiateCopy(initiateCopy);
                    frame = _activeStage.ProcessSvcProcessorMessageCliprdr(svgMessage);
                }
                else if (messageType == ClipboardMessageType.SendInitiatePaste)
                {
                    var initiatePaste = message.GetSendInitiatePaste()!;
                    var svgMessage = clipBoard.InitiatePaste(initiatePaste);
                    frame = _activeStage.ProcessSvcProcessorMessageCliprdr(svgMessage);
                }
                else
                {
                    Console.WriteLine("Error in clipboard");
                    break;
                }

                var toWriteBack = new byte[frame.GetSize()];
                frame.Fill(toWriteBack);

                await _framed!.Write(toWriteBack);
            }
        });
    }

    private static Config BuildConfig(string username, string password, string domain, int width, int height)
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

    private void Canvas_OnPointerPressed(object sender, PointerPressedEventArgs e)
    {
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
        var fastpath = _inputDatabase!.Apply(buttonOperation);
        var output = _activeStage!.ProcessFastpathInput(_decodedImage!, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_PointerMoved(object sender, PointerEventArgs e)
    {
        if (this._activeStage == null || this._decodedImage == null)
        {
            return;
        }

        var position = e.GetPosition((Visual?)sender);
        var x = (ushort)position.X;
        var y = (ushort)position.Y;
        var mouseMovedEvent = MousePosition.New(x, y).AsMoveOperation();
        var fastpath = _inputDatabase!.Apply(mouseMovedEvent);
        var output = _activeStage.ProcessFastpathInput(_decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_PointerReleased(object sender, PointerReleasedEventArgs e)
    {
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
        var fastpath = _inputDatabase!.Apply(buttonOperation);
        var output = _activeStage!.ProcessFastpathInput(_decodedImage!, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_KeyDown(object? sender, KeyEventArgs? e)
    {
        if (_activeStage == null || _decodedImage == null)
        {
            return;
        }

        PhysicalKey physicalKey = e!.PhysicalKey;

        var keyOperation = Scancode.FromU16((ushort)KeyCodeMapper.GetScancode(physicalKey)!).AsOperationKeyPressed();
        var fastpath = _inputDatabase!.Apply(keyOperation);
        var output = _activeStage.ProcessFastpathInput(_decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void Canvas_KeyUp(object? sender, KeyEventArgs? e)
    {
        if (this._activeStage == null || this._decodedImage == null)
        {
            return;
        }

        Key key = e!.Key;
        var keyOperation = Scancode.FromU16((ushort)key).AsOperationKeyReleased();
        var fastpath = _inputDatabase!.Apply(keyOperation);
        var output = _activeStage.ProcessFastpathInput(_decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private async Task<bool> HandleActiveStageOutput(ActiveStageOutputIterator outputIterator)
    {
        try
        {
            while (!outputIterator.IsEmpty())
            {
                var output =
                    outputIterator.Next()!; // outputIterator.Next() is not null since outputIterator.IsEmpty() is false
                if (output.GetEnumType() == ActiveStageOutputType.Terminate)
                {
                    return false;
                }
                else if (output.GetEnumType() == ActiveStageOutputType.ResponseFrame)
                {
                    // render the decoded image to canvas
                    WriteDecodedImageToCanvas();
                    // Send the response frame to the server
                    var responseFrame = output.GetResponseFrame();
                    byte[] responseFrameBytes = new byte[responseFrame.GetSize()];
                    responseFrame.Fill(responseFrameBytes);
                    await _framed!.Write(responseFrameBytes);
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
            }

            return true;
        }
        catch (Exception)
        {
            return false;
        }
    }

    IntPtr? GetWindowHandle()
    {
        var handle = this.TryGetPlatformHandle();
        if (handle == null)
        {
            return null;
        }

        return handle.Handle;
    }
}