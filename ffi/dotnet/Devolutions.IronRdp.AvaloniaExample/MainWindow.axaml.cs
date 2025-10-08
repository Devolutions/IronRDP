using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
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
    Framed<Stream>? _framed;  // Changed to Stream to support both SslStream and WebSocketStream
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
        var domain = Environment.GetEnvironmentVariable("IRONRDP_DOMAIN"); // Optional
        var server = Environment.GetEnvironmentVariable("IRONRDP_SERVER");
        var portEnv = Environment.GetEnvironmentVariable("IRONRDP_PORT");

        // Gateway configuration (optional)
        var gatewayUrl = Environment.GetEnvironmentVariable("IRONRDP_GATEWAY_URL");
        var gatewayToken = Environment.GetEnvironmentVariable("IRONRDP_GATEWAY_TOKEN");
        var tokengenUrl = Environment.GetEnvironmentVariable("IRONRDP_TOKENGEN_URL");

        //  KDC proxy configuration (optional)
        var kdcProxyUrlBase = Environment.GetEnvironmentVariable("IRONRDP_KDC_PROXY_URL");
        var kdcRealm = Environment.GetEnvironmentVariable("IRONRDP_KDC_REALM");
        var kdcServer = Environment.GetEnvironmentVariable("IRONRDP_KDC_SERVER");

        if (username == null || password == null || server == null)
        {
            var errorMessage =
                "Please set the IRONRDP_USERNAME, IRONRDP_PASSWORD, and IRONRDP_SERVER environment variables";
            Trace.TraceError(errorMessage);
            Close();
            throw new InvalidProgramException(errorMessage);
        }

        // Validate server is only domain or IP (no port allowed)
        // i.e. "example.com" or "10.10.0.3" the port should go to the dedicated env var IRONRDP_PORT
        if (server.Contains(':'))
        {
            var errorMessage = $"IRONRDP_SERVER must be a domain or IP address only, not '{server}'. Use IRONRDP_PORT for the port.";
            Trace.TraceError(errorMessage);
            Close();
            throw new InvalidProgramException(errorMessage);
        }

        // Parse port from environment variable or use default
        int port = 3389;
        if (!string.IsNullOrEmpty(portEnv))
        {
            if (!int.TryParse(portEnv, out port) || port <= 0 || port > 65535)
            {
                var errorMessage = $"IRONRDP_PORT must be a valid port number (1-65535), got '{portEnv}'";
                Trace.TraceError(errorMessage);
                Close();
                throw new InvalidProgramException(errorMessage);
            }
        }

        Trace.TraceInformation($"Target server: {server}:{port}");

        var config = BuildConfig(username, password, domain, _renderModel.Width, _renderModel.Height);

        CliprdrBackendFactory? factory = null;

        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            _cliprdr = WinCliprdr.New();

            if (_cliprdr != null)
            {
                factory = _cliprdr.BackendFactory();
            }
        }

        BeforeConnectSetup();
        Task.Run(async () =>
        {
            try
            {
                ConnectionResult res;

                // Determine connection mode: Gateway or Direct
                if (!string.IsNullOrEmpty(gatewayUrl))
                {
                    Trace.TraceInformation("=== GATEWAY MODE ===");
                    Trace.TraceInformation($"Gateway URL: {gatewayUrl}");
                    Trace.TraceInformation($"Destination: {server}:{port}");

                    var tokenGen = new TokenGenerator(tokengenUrl ?? "http://localhost:8080");

                    // Generate RDP token if not provided
                    if (string.IsNullOrEmpty(gatewayToken))
                    {
                        Trace.TraceInformation("No RDP token provided, generating token...");

                        try
                        {
                            gatewayToken = await tokenGen.GenerateRdpTlsToken(
                                dstHost: server!,
                                proxyUser: string.IsNullOrEmpty(domain) ? username : $"{username}@{domain}",
                                proxyPassword: password!,
                                destUser: username!,
                                destPassword: password!
                            );
                            Trace.TraceInformation($"RDP token generated successfully (length: {gatewayToken.Length})");
                        }
                        catch (Exception ex)
                        {
                            Trace.TraceError($"Failed to generate RDP token: {ex.Message}");
                            Trace.TraceInformation("Make sure tokengen server is running:");
                            Trace.TraceInformation($"  cargo run --manifest-path tools/tokengen/Cargo.toml -- server");
                            throw;
                        }
                    }

                    // Generate KDC token if KDC proxy is enabled
                    string? kdcProxyUrl = null;
                    if (!string.IsNullOrEmpty(kdcRealm) && !string.IsNullOrEmpty(kdcServer))
                    {
                        Trace.TraceInformation("=== KDC PROXY MODE ENABLED ===");
                        Trace.TraceInformation($"KDC Realm: {kdcRealm}");
                        Trace.TraceInformation($"KDC Server: {kdcServer}");

                        try
                        {
                            var kdcToken = await tokenGen.GenerateKdcToken(
                                krbRealm: kdcRealm!,
                                krbKdc: kdcServer!
                            );
                            Trace.TraceInformation($"KDC token generated successfully (length: {kdcToken.Length})");

                            // Build KDC proxy URL - use explicit URL if provided, otherwise auto-construct from gateway URL
                            if (!string.IsNullOrEmpty(kdcProxyUrlBase))
                            {
                                kdcProxyUrl = $"{kdcProxyUrlBase.TrimEnd('/')}/{kdcToken}";
                                Trace.TraceInformation($"Using explicit KDC Proxy URL: {kdcProxyUrl}");
                            }
                            else
                            {
                                var gatewayBaseUrl = new Uri(gatewayUrl.Replace("/jet/rdp", "")).GetLeftPart(UriPartial.Authority);
                                kdcProxyUrl = $"{gatewayBaseUrl}/KdcProxy/{kdcToken}";
                                Trace.TraceInformation($"Auto-constructed KDC Proxy URL: {kdcProxyUrl}");
                            }
                        }
                        catch (Exception ex)
                        {
                            Trace.TraceError($"Failed to generate KDC token: {ex.Message}");
                            Trace.TraceWarning("Continuing without KDC proxy...");
                        }
                    }

                    // Connect via gateway - destination needs "hostname:port" format for RDCleanPath
                    string destination = $"{server}:{port}";

                    // Get client hostname for Kerberos authentication
                    string? kdcClientHostname = null;
                    if (!string.IsNullOrEmpty(kdcProxyUrl))
                    {
                        kdcClientHostname = System.Net.Dns.GetHostName();
                        Trace.TraceInformation($"Client hostname for Kerberos: {kdcClientHostname}");
                    }

                    var (gatewayRes, gatewayFramed) = await GatewayConnection.ConnectViaGateway(
                        config, gatewayUrl, gatewayToken!, destination, null, factory, kdcProxyUrl, kdcClientHostname);
                    res = gatewayRes;
                    this._framed = new Framed<Stream>(gatewayFramed.GetInner().Item1);

                    Trace.TraceInformation("=== GATEWAY CONNECTION SUCCESSFUL ===");
                }
                else
                {
                    Trace.TraceInformation("=== DIRECT MODE ===");

                    // Direct connection (original behavior)
                    var (directRes, directFramed) = await Connection.Connect(config, server, factory, port);
                    res = directRes;
                    this._framed = new Framed<Stream>(directFramed.GetInner().Item1);

                    Trace.TraceInformation("=== DIRECT CONNECTION SUCCESSFUL ===");
                }

                this._decodedImage = DecodedImage.New(PixelFormat.RgbA32, res.GetDesktopSize().GetWidth(),
                    res.GetDesktopSize().GetHeight());
                this._activeStage = ActiveStage.New(res);
                ReadPduAndProcessActiveStage();

                if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                {
                    HandleClipboardEvents();
                }
            }
            catch (Exception ex)
            {
                Trace.TraceError($"Connection failed: {ex.Message}");
                Trace.TraceError($"Stack trace: {ex.StackTrace}");
                throw;
            }
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
                    var newBitmap =
                        new WriteableBitmap(new PixelSize(_decodedImage.GetWidth(), _decodedImage.GetHeight()),
                            new Vector(96, 96), Avalonia.Platform.PixelFormat.Rgba8888, AlphaFormat.Opaque);
                    _imageControl.Source = newBitmap;
                    writableBitmap = newBitmap;
                }

                using (var bitmap = writableBitmap.Lock())
                {
                    unsafe
                    {
                        var bitmapSpan = new Span<byte>((void*)bitmap.Address,
                            bitmap.Size.Width * bitmap.Size.Height * (bitmap.Format.BitsPerPixel / 8));
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

    private static Config BuildConfig(string username, string password, string? domain, int width, int height)
    {
        ConfigBuilder configBuilder = ConfigBuilder.New();

        configBuilder.WithUsernameAndPassword(username, password);
        if (domain != null)
        {
            configBuilder.SetDomain(domain);
        }

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
                        await Connection.SingleSequenceStep(activationSequence, writeBuf, _framed!);

                        if (activationSequence.GetState().GetType() != ConnectionActivationStateType.Finalized)
                            continue;

                        var finalized = activationSequence.GetState().GetFinalized();
                        var desktopSize = finalized.GetDesktopSize();
                        var ioChannelId = finalized.GetIoChannelId();
                        var userChannelId = finalized.GetUserChannelId();
                        var enableServerPointer = finalized.GetEnableServerPointer();
                        var pointerSoftwareRendering = finalized.GetPointerSoftwareRendering();

                        _decodedImage = DecodedImage.New(PixelFormat.RgbA32, desktopSize.GetWidth(),
                            desktopSize.GetHeight());

                        _activeStage!.SetFastpathProcessor(
                            ioChannelId,
                            userChannelId,
                            enableServerPointer,
                            pointerSoftwareRendering
                        );

                        _activeStage.SetEnableServerPointer(enableServerPointer);

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
