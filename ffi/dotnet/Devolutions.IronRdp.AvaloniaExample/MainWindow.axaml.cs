using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Net.Security;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Avalonia.Markup.Xaml;
using Avalonia.Interactivity;
using Image = Avalonia.Controls.Image;

namespace Devolutions.IronRdp.AvaloniaExample;

public partial class MainWindow : Window
{
    readonly InputDatabase? _inputDatabase = InputDatabase.New();
    ActiveStage? _activeStage;
    DecodedImage? _decodedImage;
    Framed<SslStream>? _framed;
    WinCliprdr? _cliprdr;
    private readonly RendererModel _renderModel;
    private Image? _imageControl;

    public MainWindow()
    {
        InitializeComponent();
        Opened += OnOpened;

        _renderModel = new RendererModel()
        {
            Width = 980,
            Height = 780
        };

        this.DataContext = _renderModel;

        Closing += (sender, e) => { Environment.Exit(1); };
    }

    private void InitializeComponent()
    {
        AvaloniaXamlLoader.Load(this);
    }

    bool _resizeTaskStarted = false;

    protected override void OnSizeChanged(SizeChangedEventArgs e)
    {
        base.OnSizeChanged(e);
        _renderModel.Width = (int)e.NewSize.Width;
        _renderModel.Height = (int)e.NewSize.Height - 100;
    }

    private void Resize(double updatedWidth, double updatedHeight)
    {
        if (_activeStage != null)
        {
            var output = _activeStage.EncodedResize((uint)updatedWidth, (uint)updatedHeight);
            if (output == null)
            {
                return;
            }

            Task.Run(async () => { await HandleActiveStageOutput(output); });
        }
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
            var errorMessage = "Please set the IRONRDP_USERNAME, IRONRDP_PASSWORD, IRONRDP_DOMAIN, and RONRDP_SERVER environment variables";
            Trace.TraceError(errorMessage);
            Close();
            throw new InvalidProgramException(errorMessage);
        }

        var config = BuildConfig(username, password, domain, _renderModel.Width, _renderModel.Height);

        CliprdrBackendFactory? factory = null;
        var handle = GetWindowHandle();
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows) && handle != null)
        {
            _cliprdr = WinCliprdr.New(handle.Value);
            if (_cliprdr != null)
            {
                factory = _cliprdr.BackendFactory();
            }
        }

        BeforeConnectSetup();
        Task.Run(async () =>
        {
            var (res, framed) = await Connection.Connect(config, server, factory);
            this._decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(),
                res.GetDesktopSize().GetHeight());
            this._activeStage = ActiveStage.New(res);
            this._framed = framed;
            ReadPduAndProcessActiveStage();
            HandleClipboardEvents();
        });

    }

    private void BeforeConnectSetup()
    {
        _imageControl = this.FindControl<Image>("RdpImage");
        if (_imageControl == null)
        {
            Trace.TraceError("Error finding Image control");
            throw new NullReferenceException("image control not found");
        }

        var bitmap = new WriteableBitmap(new PixelSize(_renderModel.Width, _renderModel.Height),
            new Vector(96, 96),
            Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque
        );
        _imageControl.Source = bitmap;
        _imageControl.SizeChanged += (sender, e) => { Resize(e.NewSize.Width, e.NewSize.Height); };
    }

    private void Render()
    {
        Dispatcher.UIThread.Invoke(() =>
        {
            try
            {

                var data = _decodedImage!.GetData();
                var bufferSize = (int)data.GetSize();

                var buffer = new byte[bufferSize];
                data.Fill(buffer);

                if (_imageControl is not { Source: WriteableBitmap writableBitmap })
                {
                    return;
                }

                var currentBitmapSize = writableBitmap.Size.Width * writableBitmap.Size.Height * 4;
                if (Math.Abs(bufferSize - currentBitmapSize) > 1)
                {
                    var newBitmap = new WriteableBitmap(new PixelSize(_decodedImage.GetWidth(), _decodedImage.GetHeight()), new Vector(96, 96), Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque);
                    _imageControl.Source = newBitmap;
                    writableBitmap = newBitmap;
                }

                using (var bitmap = writableBitmap.Lock())
                {
                    unsafe
                    {
                        var bitmapSpan = new Span<byte>((void*)bitmap.Address, bitmap.Size.Width * bitmap.Size.Height * (bitmap.Format.BitsPerPixel / 8));
                        bitmapSpan.Clear();
                        var bufferSpan = new Span<byte>(buffer);
                        if (bufferSize > bitmapSpan.Length)
                        {
                            throw new InvalidOperationException("buffer size does not match");
                        }
                        else
                        {
                            bufferSpan.CopyTo(bitmapSpan);
                        }
                    }
                }
                _imageControl!.InvalidateVisual();
            }
            catch
            {
                Trace.TraceError("error rendering");
            }
        });
    }


    private void ReadPduAndProcessActiveStage()
    {
        Task.Run(async () =>
        {
            try
            {
                var keepLooping = true;
                while (keepLooping)
                {
                    var (action, payload) = await _framed!.ReadPdu();
                    var outputIterator = _activeStage!.Process(_decodedImage!, action, payload);
                    keepLooping = await HandleActiveStageOutput(outputIterator);
                }

                Trace.TraceInformation("ReadPduAndProcessActiveStage loop ended");
            }
            catch (Exception e)
            {
                Trace.TraceError("Error reading PDU: " + e.Message);
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

                VecU8 frame;
                var messageType = message.GetMessageType();
                Trace.TraceInformation("Clipboard message type: " + messageType);
                if (messageType == ClipboardMessageType.SendFormatData)
                {
                    var formatData = message.GetSendFormatData()!;
                    frame = _activeStage!.SubmitClipboardFormatData(formatData);
                }
                else if (messageType == ClipboardMessageType.SendInitiateCopy)
                {
                    var initiateCopy = message.GetSendInitiateCopy()!;
                    frame = _activeStage!.InitiateClipboardCopy(initiateCopy);
                }
                else if (messageType == ClipboardMessageType.SendInitiatePaste)
                {
                    var initiatePaste = message.GetSendInitiatePaste()!;
                    frame = _activeStage!.InitiateClipboardPaste(initiatePaste);
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

    private void OnPointerPressed(object sender, PointerPressedEventArgs e)
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

    private void OnPointerMoved(object sender, PointerEventArgs e)
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

    private void OnPointerReleased(object sender, PointerReleasedEventArgs e)
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

    private void OnKeyDown(object? sender, KeyEventArgs? e)
    {
        if (_activeStage == null || _decodedImage == null)
        {
            return;
        }

        PhysicalKey physicalKey = e!.PhysicalKey;

        var keycode = KeyCodeMapper.GetScancode(physicalKey);

        if (keycode == null)
        {
            return;
        }

        var keyOperation = Scancode.FromU16(keycode.Value).AsOperationKeyPressed();
        var fastpath = _inputDatabase!.Apply(keyOperation);
        var output = _activeStage.ProcessFastpathInput(_decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private void OnKeyUp(object? sender, KeyEventArgs? e)
    {
        if (_activeStage == null || _decodedImage == null)
        {
            return;
        }

        PhysicalKey physicalKey = e!.PhysicalKey;

        var keycode = KeyCodeMapper.GetScancode(physicalKey);

        if (keycode == null)
        {
            return;
        }

        var keyOperation = Scancode.FromU16(keycode.Value).AsOperationKeyReleased();
        var fastpath = _inputDatabase!.Apply(keyOperation);
        var output = _activeStage.ProcessFastpathInput(_decodedImage, fastpath);
        var _ = HandleActiveStageOutput(output);
    }

    private async Task<bool> HandleActiveStageOutput(ActiveStageOutputIterator outputIterator)
    {
        while (!outputIterator.IsEmpty())
        {
            try
            {
                var output =
                    outputIterator
                        .Next()!; // outputIterator.Next() is not null since outputIterator.IsEmpty() is false
                if (output.GetEnumType() == ActiveStageOutputType.Terminate)
                {
                    return false;
                }

                if (output.GetEnumType() == ActiveStageOutputType.ResponseFrame)
                {
                    // Send the response frame to the server
                    var responseFrame = output.GetResponseFrame();
                    byte[] responseFrameBytes = new byte[responseFrame.GetSize()];
                    responseFrame.Fill(responseFrameBytes);
                    await _framed!.Write(responseFrameBytes);
                }
                else if (output.GetEnumType() == ActiveStageOutputType.GraphicsUpdate)
                {
                    Render();
                }
                else if (output.GetEnumType() == ActiveStageOutputType.DeactivateAll)
                {
                    var activationSequence = output.GetDeactivateAll();
                    var writeBuf = WriteBuf.New();
                    while (true)
                    {
                        var written = await Connection.SingleSequenceStepRead(_framed!, activationSequence, writeBuf);
                        if (written.GetSize().IsSome())
                        {
                            await _framed!.Write(writeBuf);
                        }

                        if (activationSequence.GetState().GetType() != ConnectionActivationStateType.Finalized)
                            continue;

                        var finalized = activationSequence.GetState().GetFinalized();
                        var desktopSize = finalized.GetDesktopSize();
                        var ioChannelId = finalized.GetIoChannelId();
                        var userChannelId = finalized.GetUserChannelId();
                        var noServerPointer = finalized.GetNoServerPointer();
                        var pointerSoftwareRendering = finalized.GetPointerSoftwareRendering();

                        _decodedImage = DecodedImage.New(PixelFormat.RgbA32, desktopSize.GetWidth(),
                            desktopSize.GetHeight());

                        _activeStage!.SetFastpathProcessor(
                            ioChannelId,
                            userChannelId,
                            noServerPointer,
                            pointerSoftwareRendering
                        );

                        _activeStage.SetNoServerPointer(noServerPointer);

                        break;
                    }
                }
                else
                {
                    Trace.TraceError("Unhandled ActiveStageOutputType: " + output.GetEnumType());
                }
            }
            catch (Exception e)
            {
                Trace.TraceError("Error processing active stage output: " + e.Message);
            }
        }

        return true;
    }

    IntPtr? GetWindowHandle()
    {
        var handle = TryGetPlatformHandle();
        if (handle == null)
        {
            return null;
        }

        return handle.Handle;
    }

    public void OnDisconnectClick(object? sender, RoutedEventArgs e)
    {
        var output = this._activeStage!.GracefulShutdown();

        HandleActiveStageOutput(output).ContinueWith(t =>
        {
            if (t.IsFaulted)
            {
                Trace.TraceError("Error processing active stage: " + t.Exception!.Message);
            }
        });
    }

    private void OnCtrlAltDelClick(object? sender, RoutedEventArgs e)
    {
        var ctrlScanCode = KeyCodeMapper.GetScancode(PhysicalKey.ControlLeft);
        var altScanCode = KeyCodeMapper.GetScancode(PhysicalKey.AltLeft);
        var delScanCode = KeyCodeMapper.GetScancode(PhysicalKey.Delete);

        if (ctrlScanCode == null || altScanCode == null || delScanCode == null)
        {
            Trace.TraceError("Error getting scancodes for Ctrl, Alt, and Del keys");
            throw new ApplicationException("should not happen, check KeyCodeMapper.cs");
        }

        var ctrlOperation = Scancode.FromU16(ctrlScanCode.Value).AsOperationKeyPressed();
        var altOperation = Scancode.FromU16(altScanCode.Value).AsOperationKeyPressed();
        var delOperation = Scancode.FromU16(delScanCode.Value).AsOperationKeyPressed();

        var ctrlFastpath = _inputDatabase!.Apply(ctrlOperation);
        var altFastpath = _inputDatabase!.Apply(altOperation);
        var delFastpath = _inputDatabase!.Apply(delOperation);

        var ctrlOutput = _activeStage!.ProcessFastpathInput(_decodedImage, ctrlFastpath);
        var altOutput = _activeStage!.ProcessFastpathInput(_decodedImage, altFastpath);
        var delOutput = _activeStage!.ProcessFastpathInput(_decodedImage, delFastpath);

        Task.Run(async () =>
        {
            await HandleActiveStageOutput(ctrlOutput);
            await HandleActiveStageOutput(altOutput);
            await HandleActiveStageOutput(delOutput);
        });
    }
}

public sealed class RendererModel : INotifyPropertyChanged
{
    private int _width;
    private int _height;

    public int Width
    {
        get => _width;
        set
        {
            if (_width != value)
            {
                _width = value;
                OnPropertyChanged();
            }
        }
    }

    public int Height
    {
        get { return _height; }
        set
        {
            if (_height != value)
            {
                _height = value;
                OnPropertyChanged();
            }
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null)
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}