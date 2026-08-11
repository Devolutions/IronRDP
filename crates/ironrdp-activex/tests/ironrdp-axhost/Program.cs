using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text.Json;
using System.Windows.Forms;
using Microsoft.Win32.SafeHandles;

internal sealed class DirectActiveXHost : AxHost
{
    private static readonly object RetainedModulesLock = new();
    private static readonly List<ComActivation.SafeLibraryHandle> RetainedModules = new();

    private readonly ComActivation.SafeLibraryHandle module;
    private bool hostDisposed;
    private bool moduleLifetimeHandled;

    internal DirectActiveXHost(Guid classId, string libraryPath)
        : base(classId.ToString("B"))
    {
        module = ComActivation.LoadLibrary(libraryPath);
    }

    protected override object CreateInstanceCore(Guid classId) => ComActivation.CreateInstance(module, classId);

    internal object ActiveXObject => GetOcx();

    internal int DisposeAndGetUnloadStatus()
    {
        DisposeHost();
        int unloadStatus = ComActivation.GetCanUnloadNow(module);
        if (unloadStatus == 0)
            DisposeModule();
        else
            RetainModuleUntilProcessExit();

        return unloadStatus;
    }

    protected override void Dispose(bool disposing)
    {
        if (!disposing)
        {
            base.Dispose(disposing);
            return;
        }

        DisposeHost();
        RetainModuleUntilProcessExit();
    }

    private void DisposeHost()
    {
        if (hostDisposed)
            return;

        base.Dispose(true);
        hostDisposed = true;
    }

    private void DisposeModule()
    {
        if (moduleLifetimeHandled)
            return;

        module.Dispose();
        moduleLifetimeHandled = true;
    }

    private void RetainModuleUntilProcessExit()
    {
        if (moduleLifetimeHandled)
            return;

        lock (RetainedModulesLock)
            RetainedModules.Add(module);
        moduleLifetimeHandled = true;
    }
}

internal static class ComActivation
{
    private static readonly Guid IidIClassFactory = new("00000001-0000-0000-C000-000000000046");
    private static readonly Guid IidIUnknown = new("00000000-0000-0000-C000-000000000046");

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int DllGetClassObject(ref Guid classId, ref Guid interfaceId, out nint factory);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int DllCanUnloadNow();

    internal sealed class SafeLibraryHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal SafeLibraryHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle() => FreeLibrary(handle);
    }

    [ComImport]
    [Guid("00000001-0000-0000-C000-000000000046")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IClassFactory
    {
        void CreateInstance(
            [MarshalAs(UnmanagedType.IUnknown)] object? outer,
            ref Guid interfaceId,
            [MarshalAs(UnmanagedType.IUnknown)] out object instance);

        void LockServer([MarshalAs(UnmanagedType.Bool)] bool lockServer);
    }

    [DllImport("kernel32.dll", EntryPoint = "LoadLibraryW", ExactSpelling = true, CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeLibraryHandle? LoadLibraryW(string path);

    [DllImport("kernel32.dll", EntryPoint = "GetProcAddress", ExactSpelling = true, CharSet = CharSet.Ansi, SetLastError = true)]
    private static extern nint GetProcAddress(SafeLibraryHandle module, string name);

    [DllImport("kernel32.dll", EntryPoint = "FreeLibrary", ExactSpelling = true, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool FreeLibrary(nint module);

    internal static SafeLibraryHandle LoadLibrary(string libraryPath)
    {
        SafeLibraryHandle? module = LoadLibraryW(libraryPath);
        if (module is null || module.IsInvalid)
        {
            module?.Dispose();
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Unable to load the requested COM server.");
        }

        return module;
    }

    internal static object CreateInstance(SafeLibraryHandle module, Guid classId)
    {
        bool moduleReferenceAdded = false;
        nint factoryPointer = 0;

        try
        {
            module.DangerousAddRef(ref moduleReferenceAdded);
            nint address = GetProcAddress(module, "DllGetClassObject");
            if (address == 0)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "The requested COM server does not export DllGetClassObject.");

            var getClassObject = Marshal.GetDelegateForFunctionPointer<DllGetClassObject>(address);
            var classFactoryId = IidIClassFactory;
            int result = getClassObject(ref classId, ref classFactoryId, out factoryPointer);
            Marshal.ThrowExceptionForHR(result);
            if (factoryPointer == 0)
                throw new COMException("The requested COM server returned a null class factory.", unchecked((int)0x8000FFFF));

            var factory = (IClassFactory)Marshal.GetObjectForIUnknown(factoryPointer);
            try
            {
                var unknownId = IidIUnknown;
                factory.CreateInstance(null, ref unknownId, out object instance);
                return instance;
            }
            finally
            {
                Marshal.ReleaseComObject(factory);
            }
        }
        finally
        {
            if (factoryPointer != 0)
                Marshal.Release(factoryPointer);
            if (moduleReferenceAdded)
                module.DangerousRelease();
        }
    }

    internal static int GetCanUnloadNow(SafeLibraryHandle module)
    {
        bool moduleReferenceAdded = false;
        try
        {
            module.DangerousAddRef(ref moduleReferenceAdded);
            nint address = GetProcAddress(module, "DllCanUnloadNow");
            if (address == 0)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "The requested COM server does not export DllCanUnloadNow.");

            return Marshal.GetDelegateForFunctionPointer<DllCanUnloadNow>(address)();
        }
        finally
        {
            if (moduleReferenceAdded)
                module.DangerousRelease();
        }
    }
}

[ComVisible(true)]
[Guid("336D5562-EFA8-482E-8CB3-C5C0FC7A7DB6")]
[InterfaceType(ComInterfaceType.InterfaceIsIDispatch)]
public interface IMsTscAxEvents
{
    [DispId(1)]
    void OnConnecting();

    [DispId(2)]
    void OnConnected();

    [DispId(4)]
    void OnDisconnected(int reason);

    [DispId(10)]
    void OnFatalError(int errorCode);
}

[ComImport]
[Guid("302D8188-0052-4807-806A-362B628F9AC5")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMsRdpExtendedSettings
{
    void put_Property([MarshalAs(UnmanagedType.BStr)] string name, [In] ref object value);

    void get_Property([MarshalAs(UnmanagedType.BStr)] string name, [Out] out object value);
}

internal static class RemoteProgramInterop
{
    // Slots include IUnknown, IDispatch, and all inherited published interface members.
    private const int ImRdpClient5GetRemoteProgramSlot = 55;
    private const int RemoteProgramServerStartProgramSlot = 9;
    private static readonly Guid IMsRdpClient5Id = new("4EB5335B-6429-477D-B922-E06A28ECD8BF");

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int GetRemoteProgram(nint self, out nint remoteProgram);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int ServerStartProgram(
        nint self,
        nint executable,
        nint file,
        nint workingDirectory,
        short expandWorkingDirectory,
        nint arguments,
        short expandArguments);

    internal static void StartProgram(object control, string executable)
    {
        nint unknown = Marshal.GetIUnknownForObject(control);
        nint client = 0;
        nint remoteProgram = 0;
        nint executableBstr = 0;
        nint emptyFileBstr = 0;
        nint emptyWorkingDirectoryBstr = 0;
        nint emptyArgumentsBstr = 0;

        try
        {
            Guid interfaceId = IMsRdpClient5Id;
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unknown, ref interfaceId, out client));
            if (client == 0)
                throw new InvalidOperationException("The ActiveX control returned a null IMsRdpClient5 interface.");

            var getRemoteProgram = VtableMethod<GetRemoteProgram>(client, ImRdpClient5GetRemoteProgramSlot);
            Marshal.ThrowExceptionForHR(getRemoteProgram(client, out remoteProgram));
            if (remoteProgram == 0)
                throw new InvalidOperationException("The ActiveX control returned a null RemoteProgram interface.");

            executableBstr = Marshal.StringToBSTR(executable);
            emptyFileBstr = Marshal.StringToBSTR(string.Empty);
            emptyWorkingDirectoryBstr = Marshal.StringToBSTR(string.Empty);
            emptyArgumentsBstr = Marshal.StringToBSTR(string.Empty);

            var serverStartProgram = VtableMethod<ServerStartProgram>(
                remoteProgram,
                RemoteProgramServerStartProgramSlot);
            Marshal.ThrowExceptionForHR(serverStartProgram(
                remoteProgram,
                executableBstr,
                emptyFileBstr,
                emptyWorkingDirectoryBstr,
                0,
                emptyArgumentsBstr,
                0));
        }
        finally
        {
            FreeBstr(executableBstr);
            FreeBstr(emptyFileBstr);
            FreeBstr(emptyWorkingDirectoryBstr);
            FreeBstr(emptyArgumentsBstr);
            if (remoteProgram != 0)
                Marshal.Release(remoteProgram);
            if (client != 0)
                Marshal.Release(client);
            Marshal.Release(unknown);
        }
    }

    private static T VtableMethod<T>(nint interfacePointer, int slot)
        where T : Delegate
    {
        nint vtable = Marshal.ReadIntPtr(interfacePointer);
        nint method = Marshal.ReadIntPtr(vtable, checked(slot * IntPtr.Size));
        if (method == 0)
            throw new InvalidOperationException("The requested ActiveX interface method is unavailable.");

        return Marshal.GetDelegateForFunctionPointer<T>(method);
    }

    private static void FreeBstr(nint value)
    {
        if (value != 0)
            Marshal.FreeBSTR(value);
    }
}

[ComVisible(true)]
public sealed class LifecycleSink : IMsTscAxEvents
{
    public event Action? Connecting;
    public event Action? Connected;
    public event Action? Disconnected;
    public event Action? FatalError;

    public void OnConnecting() => Connecting?.Invoke();

    public void OnConnected() => Connected?.Invoke();

    public void OnDisconnected(int reason) => Disconnected?.Invoke();

    public void OnFatalError(int errorCode) => FatalError?.Invoke();
}

internal enum Operation
{
    Probe,
    Connect,
    Unload,
}

internal sealed record ConnectionSettings(string Server, string UserName, string Password)
{
    internal const string DefaultServerVariable = "RDP_HOSTNAME";
    internal const string DefaultUserNameVariable = "RDP_USERNAME";
    internal const string DefaultPasswordVariable = "RDP_PASSWORD";

    internal static ConnectionSettings FromEnvironment(string? server, string? userName, string passwordVariable)
    {
        return new ConnectionSettings(
            server ?? RequiredEnvironmentVariable(DefaultServerVariable),
            userName ?? RequiredEnvironmentVariable(DefaultUserNameVariable),
            RequiredEnvironmentVariable(passwordVariable));
    }

    private static string RequiredEnvironmentVariable(string name)
    {
        string? value = Environment.GetEnvironmentVariable(name);
        if (string.IsNullOrWhiteSpace(value))
            throw new InvalidOperationException($"The {name} environment variable must be set.");

        return value;
    }
}

internal sealed record AxHostOptions(
    string LibraryPath,
    Guid ClassId,
    Operation Operation,
    bool AutoLogon,
    string? RemoteApplicationProgram,
    string? RemoteApplicationArgs,
    IReadOnlyList<string> RailLaunches,
    bool Show,
    bool Json,
    TimeSpan Timeout,
    TimeSpan Observe,
    string? ScreenshotPath,
    string? Server,
    string? UserName,
    string PasswordVariable,
    int DesktopWidth,
    int DesktopHeight);

internal sealed record AxHostReport(
    string Operation,
    bool Passed,
    int ExitCode,
    long DurationMilliseconds,
    long? HostHandle,
    bool ConnectedProperty,
    IReadOnlyList<string> Events,
    string? Screenshot,
    string? Failure,
    int? UnloadStatus);

internal sealed record LifecycleResult(
    bool Passed,
    bool ConnectedProperty,
    IReadOnlyList<string> Events,
    string? Screenshot,
    string? Failure);

internal static class ReportWriter
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = false,
    };

    internal static void Write(AxHostReport report, bool json)
    {
        if (json)
        {
            Console.WriteLine(JsonSerializer.Serialize(report, JsonOptions));
            return;
        }

        Console.WriteLine($"{report.Operation}: {(report.Passed ? "passed" : "failed")}");
        Console.WriteLine($"duration-ms: {report.DurationMilliseconds}");
        if (report.HostHandle is long handle)
            Console.WriteLine($"host-handle: {handle}");
        Console.WriteLine($"connected-property: {report.ConnectedProperty}");
        Console.WriteLine($"events: {(report.Events.Count == 0 ? "none" : string.Join(",", report.Events))}");
        if (report.Screenshot is not null)
            Console.WriteLine($"screenshot: {report.Screenshot}");
        if (report.Failure is not null)
            Console.Error.WriteLine($"failure: {report.Failure}");
        if (report.UnloadStatus is int unloadStatus)
            Console.WriteLine($"dll-can-unload-now: 0x{unchecked((uint)unloadStatus):X8}");
    }
}

internal sealed class LifecycleRunner
{
    private const BindingFlags DispatchFlags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.InvokeMethod;
    private const BindingFlags PropertyGetFlags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.GetProperty;
    private const BindingFlags PropertySetFlags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.SetProperty;
    private readonly object control;
    private readonly DirectActiveXHost host;
    private readonly Form form;
    private readonly AxHostOptions options;
    private readonly LifecycleSink sink = new();
    private readonly List<string> events = [];
    private IConnectionPoint? connectionPoint;
    private Timer? timer;
    private int cookie;
    private bool eventSinkAdvised;
    private bool connectInvoked;
    private bool finished;
    private bool eventHandlersAttached;
    private DateTime? observationDeadline;
    private bool connectedProperty;
    private string? screenshot;
    private string? failure;

    private bool HasRemoteAppConfiguration =>
        options.RemoteApplicationProgram is not null || options.RailLaunches.Count != 0;

    internal LifecycleRunner(object control, DirectActiveXHost host, Form form, AxHostOptions options)
    {
        this.control = control;
        this.host = host;
        this.form = form;
        this.options = options;
    }

    internal LifecycleResult Run(ConnectionSettings settings)
    {
        try
        {
            Configure(settings);
            AttachManagedEventHandlers();
            AttachEvents();
            connectInvoked = true;
            StartConnectionTimer(DateTime.UtcNow + options.Timeout);
            InvokeMethod("Connect");

            if (!finished)
            {
                if (options.Show)
                    form.Show();
                Application.Run(form);
            }
        }
        catch
        {
            Finish(false, "the connection lifecycle could not be completed");
        }
        finally
        {
            Finish(false, "connection ended before it was established");
            DetachManagedEventHandlers();
        }

        return new LifecycleResult(failure is null, connectedProperty, events.ToArray(), screenshot, failure);
    }

    private void StartConnectionTimer(DateTime deadline)
    {
        timer = new Timer { Interval = 100 };
        timer.Tick += (_, _) => OnTimer(deadline);
        timer.Start();
    }

    private void Configure(ConnectionSettings settings)
    {
        SetProperty("Server", settings.Server);
        SetProperty("UserName", settings.UserName);
        SetProperty("DesktopWidth", options.DesktopWidth);
        SetProperty("DesktopHeight", options.DesktopHeight);
        SetProperty("IronRdpPassword", settings.Password);
        if (options.AutoLogon)
            SetExtendedSetting("IronRdpAutoLogon", true);
        if (options.RemoteApplicationProgram is not null)
        {
            SetExtendedSetting("IronRdpRemoteProgramMode", true);
            SetExtendedSetting("IronRdpRemoteApplicationProgram", options.RemoteApplicationProgram);
            if (options.RemoteApplicationArgs is not null)
                SetExtendedSetting("IronRdpRemoteApplicationArgs", options.RemoteApplicationArgs);
        }
        else if (options.RailLaunches.Count != 0)
        {
            SetExtendedSetting("IronRdpRemoteProgramMode", true);
        }
    }

    private void AttachEvents()
    {
        if (control is not IConnectionPointContainer container)
            throw new InvalidOperationException("The ActiveX control does not expose IConnectionPointContainer.");

        Guid eventInterface = typeof(IMsTscAxEvents).GUID;
        IConnectionPoint? point = null;
        try
        {
            container.FindConnectionPoint(ref eventInterface, out point);
            if (point is null)
                throw new InvalidOperationException("The ActiveX control did not return its lifecycle connection point.");

            point.Advise(sink, out cookie);
            eventSinkAdvised = true;
            connectionPoint = point;
            point = null;
        }
        finally
        {
            if (point is not null)
                Marshal.ReleaseComObject(point);
        }
    }

    private void AttachManagedEventHandlers()
    {
        form.FormClosed += OnFormClosed;
        sink.Connecting += OnConnecting;
        sink.Connected += OnConnected;
        sink.Disconnected += OnDisconnected;
        sink.FatalError += OnFatalError;
        eventHandlersAttached = true;
    }

    private void DetachManagedEventHandlers()
    {
        if (!eventHandlersAttached)
            return;

        form.FormClosed -= OnFormClosed;
        sink.Connecting -= OnConnecting;
        sink.Connected -= OnConnected;
        sink.Disconnected -= OnDisconnected;
        sink.FatalError -= OnFatalError;
        eventHandlersAttached = false;
    }

    private void OnConnecting()
    {
        if (!finished)
            events.Add("connecting");
    }

    private void OnDisconnected()
    {
        if (finished)
            return;

        events.Add("disconnected");
        Finish(false, "disconnected before connection completed");
    }

    private void OnFatalError()
    {
        if (finished)
            return;

        events.Add("fatal-error");
        Finish(false, "the control reported a fatal error");
    }

    private void OnTimer(DateTime deadline)
    {
        if (IsConnected())
        {
            connectedProperty = true;
            OnConnected();
            return;
        }

        if (DateTime.UtcNow >= deadline)
            Finish(false, "timed out waiting for a connected state");
    }

    private void OnConnected()
    {
        if (finished || observationDeadline is not null)
            return;

        connectedProperty = true;
        events.Add("connected");
        try
        {
            foreach (string executable in options.RailLaunches)
            {
                StartRemoteProgram(executable);
                events.Add("rail-launch-dispatched");
            }
            if (options.Show && HasRemoteAppConfiguration)
            {
                // Projected RAIL windows are independent top-level HWNDs. Once connected, the
                // AxHost form is only the unused desktop canvas and would otherwise show as black.
                form.Hide();
                events.Add("remoteapp-host-hidden");
            }
        }
        catch (COMException)
        {
            Finish(false, "the ActiveX control rejected the requested RAIL launch");
            return;
        }
        catch (InvalidOperationException)
        {
            Finish(false, "could not dispatch the requested RAIL launch");
            return;
        }

        if (options.ScreenshotPath is not null)
        {
            try
            {
                CaptureScreenshot(options.ScreenshotPath);
                screenshot = Path.GetFullPath(options.ScreenshotPath);
            }
            catch
            {
                Finish(false, "could not capture the requested screenshot");
                return;
            }
        }

        if (options.Observe == TimeSpan.Zero)
        {
            Finish(true, null);
            return;
        }

        DateTime deadline = DateTime.UtcNow + options.Observe;
        observationDeadline = deadline;
        DisposeTimer();
        StartObservationTimer(deadline);
    }

    private void StartObservationTimer(DateTime deadline)
    {
        timer = new Timer { Interval = 100 };
        timer.Tick += (_, _) =>
        {
            if (DateTime.UtcNow >= deadline)
                Finish(true, null);
        };
        timer.Start();
    }

    private void CaptureScreenshot(string path)
    {
        if (host.Width <= 0 || host.Height <= 0)
            throw new InvalidOperationException("the AxHost does not have a drawable size");

        string? directory = Path.GetDirectoryName(Path.GetFullPath(path));
        if (!string.IsNullOrEmpty(directory))
            Directory.CreateDirectory(directory);

        using var bitmap = new Bitmap(host.Width, host.Height);
        host.DrawToBitmap(bitmap, new Rectangle(Point.Empty, bitmap.Size));
        bitmap.Save(path, ImageFormat.Png);
    }

    private void OnFormClosed(object? sender, FormClosedEventArgs eventArgs)
    {
        if (!finished)
            Finish(false, "the AxHost window closed before connection completed");
    }

    private void Finish(bool success, string? message)
    {
        if (finished)
            return;

        finished = true;
        failure = success ? null : message;
        DisposeTimer();

        if (connectInvoked)
        {
            try
            {
                InvokeMethod("Disconnect");
            }
            catch
            {
                if (success)
                    failure = "Disconnect failed during cleanup";
            }
        }

        if (eventSinkAdvised && connectionPoint is not null)
        {
            try
            {
                connectionPoint.Unadvise(cookie);
            }
            catch
            {
                if (success)
                    failure = "the lifecycle event sink could not be detached";
            }
            eventSinkAdvised = false;
            cookie = 0;
        }

        if (connectionPoint is not null)
        {
            try
            {
                Marshal.ReleaseComObject(connectionPoint);
            }
            catch
            {
                if (success)
                    failure = "the lifecycle connection point could not be released";
            }
            finally
            {
                connectionPoint = null;
            }
        }

        if (!form.IsDisposed)
            form.Close();
    }

    private void DisposeTimer()
    {
        if (timer is null)
            return;

        timer.Stop();
        timer.Dispose();
        timer = null;
    }

    private bool IsConnected()
    {
        try
        {
            object? value = GetProperty("Connected");
            return value is not null && Convert.ToBoolean(value);
        }
        catch
        {
            return false;
        }
    }

    private object? GetProperty(string name) =>
        control.GetType().InvokeMember(name, PropertyGetFlags, null, control, null);

    private void SetProperty(string name, object value) =>
        control.GetType().InvokeMember(name, PropertySetFlags, null, control, [value]);

    private void SetExtendedSetting(string name, object value)
    {
        nint unknown = Marshal.GetIUnknownForObject(control);
        nint extendedSettingsPointer = 0;
        try
        {
            Guid interfaceId = typeof(IMsRdpExtendedSettings).GUID;
            Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unknown, ref interfaceId, out extendedSettingsPointer));
            var extendedSettings = (IMsRdpExtendedSettings)Marshal.GetObjectForIUnknown(extendedSettingsPointer);
            extendedSettings.put_Property(name, ref value);
        }
        finally
        {
            if (extendedSettingsPointer != 0)
                Marshal.Release(extendedSettingsPointer);
            Marshal.Release(unknown);
        }
    }

    private void StartRemoteProgram(string executable)
        => RemoteProgramInterop.StartProgram(control, executable);

    private void InvokeMethod(string name) =>
        control.GetType().InvokeMember(name, DispatchFlags, null, control, null);
}

internal static class Program
{
    private static readonly Guid IronRdpClassId = new("5D3E2B4C-6860-462E-8E9D-0C4D2B094C5F");
    private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(30);
    private const string DefaultPasswordVariable = ConnectionSettings.DefaultPasswordVariable;

    [STAThread]
    private static int Main(string[] args)
    {
        if (args.Length == 1 && args[0] == "--help-agent")
        {
            Console.Write(AgentGuide);
            return 0;
        }

        AxHostOptions? options = null;
        bool jsonRequested = Array.IndexOf(args, "--json") >= 0;
        var stopwatch = Stopwatch.StartNew();
        try
        {
            options = ParseOptions(args);
            Application.OleRequired();
            using var form = new Form
            {
                Text = "IronRDP AxHost",
                ClientSize = new Size(1024, 768),
                StartPosition = FormStartPosition.CenterScreen,
            };
            var host = new DirectActiveXHost(options.ClassId, options.LibraryPath) { Dock = DockStyle.Fill };
            try
            {
                var initialization = (ISupportInitialize)host;
                initialization.BeginInit();
                form.Controls.Add(host);
                initialization.EndInit();
                form.CreateControl();
                host.CreateControl();
                nint hostHandle = host.Handle;

                LifecycleResult result = options.Operation switch
                {
                    Operation.Probe => RunProbe(host, form, options),
                    Operation.Connect => new LifecycleRunner(
                        host.ActiveXObject,
                        host,
                        form,
                        options).Run(ConnectionSettings.FromEnvironment(options.Server, options.UserName, options.PasswordVariable)),
                    Operation.Unload => RunUnloadProbe(host),
                    _ => throw new InvalidOperationException("unknown test operation"),
                };

                int? unloadStatus = null;
                if (options.Operation == Operation.Unload)
                {
                    unloadStatus = host.DisposeAndGetUnloadStatus();
                    if (unloadStatus != 0)
                    {
                        result = new LifecycleResult(
                            false,
                            result.ConnectedProperty,
                            result.Events,
                            result.Screenshot,
                            "the COM server remained locked after AxHost teardown");
                    }
                }
                else
                {
                    host.Dispose();
                }

                return Complete(options, stopwatch, result, hostHandle, unloadStatus);
            }
            finally
            {
                host.Dispose();
            }
        }
        catch (ArgumentException)
        {
            if (options?.Json == true || jsonRequested)
                ReportWriter.Write(FailureReport("parse", stopwatch, "invalid command line", 64), true);
            else
                PrintUsage();
            return 64;
        }
        catch (InvalidOperationException exception)
        {
            return CompleteFailure(options, stopwatch, "setup", DescribeSetupFailure(exception));
        }
        catch (Exception exception)
        {
            return CompleteFailure(options, stopwatch, "setup", DescribeSetupFailure(exception));
        }
    }

    private static LifecycleResult RunProbe(DirectActiveXHost host, Form form, AxHostOptions options)
    {
        _ = host.ActiveXObject;
        if (options.Show)
        {
            using var timer = new Timer { Interval = (int)options.Observe.TotalMilliseconds };
            timer.Tick += (_, _) => form.Close();
            timer.Start();
            form.Show();
            Application.Run(form);
        }

        return new LifecycleResult(true, false, ["activated"], null, null);
    }

    private static LifecycleResult RunUnloadProbe(DirectActiveXHost host)
    {
        object control = host.ActiveXObject;
        if (control is not IConnectionPointContainer container)
            throw new InvalidOperationException("The ActiveX control does not expose IConnectionPointContainer.");

        Guid eventInterface = typeof(IMsTscAxEvents).GUID;
        container.FindConnectionPoint(ref eventInterface, out IConnectionPoint? point);
        if (point is null)
            throw new InvalidOperationException("The ActiveX control did not return its lifecycle connection point.");

        try
        {
            var sink = new LifecycleSink();
            point.Advise(sink, out int cookie);
            point.Unadvise(cookie);
        }
        finally
        {
            Marshal.ReleaseComObject(point);
        }

        // AxHost owns the root control RCW. Releasing it here separates the object before AxHost
        // processes its final window messages; DisposeAndGetUnloadStatus performs that release.
        return new LifecycleResult(true, false, ["activated", "advised", "unadvised"], null, null);
    }

    private static int Complete(
        AxHostOptions options,
        Stopwatch stopwatch,
        LifecycleResult result,
        nint handle,
        int? unloadStatus)
    {
        stopwatch.Stop();
        var report = new AxHostReport(
            options.Operation.ToString().ToLowerInvariant(),
            result.Passed,
            result.Passed ? 0 : 1,
            stopwatch.ElapsedMilliseconds,
            handle == 0 ? null : (long)handle,
            result.ConnectedProperty,
            result.Events,
            result.Screenshot,
            result.Failure,
            unloadStatus);
        ReportWriter.Write(report, options.Json);
        return report.ExitCode;
    }

    private static int CompleteFailure(AxHostOptions? options, Stopwatch stopwatch, string operation, string failure)
    {
        stopwatch.Stop();
        bool json = options?.Json == true;
        ReportWriter.Write(FailureReport(operation, stopwatch, failure), json);
        return 1;
    }

    private static AxHostReport FailureReport(string operation, Stopwatch stopwatch, string failure, int exitCode = 1) =>
        new(operation, false, exitCode, stopwatch.ElapsedMilliseconds, null, false, [], null, failure, null);

    private static string DescribeSetupFailure(Exception exception) =>
        $"the AxHost test could not be completed ({exception.GetType().Name})";

    private static AxHostOptions ParseOptions(string[] args)
    {
        if (args.Length == 0 || string.IsNullOrWhiteSpace(args[0]) || args[0].StartsWith("-", StringComparison.Ordinal))
            throw new ArgumentException();

        string libraryPath = Path.GetFullPath(args[0]);
        Guid classId = IronRdpClassId;
        Operation operation = Operation.Probe;
        bool operationSpecified = false;
        bool classIdSpecified = false;
        bool autoLogon = false;
        bool remoteApplicationProgramSpecified = false;
        bool remoteApplicationArgsSpecified = false;
        bool timeoutSpecified = false;
        bool observeSpecified = false;
        bool screenshotSpecified = false;
        bool serverSpecified = false;
        bool userNameSpecified = false;
        bool passwordVariableSpecified = false;
        bool desktopWidthSpecified = false;
        bool desktopHeightSpecified = false;
        bool show = false;
        bool json = false;
        TimeSpan timeout = DefaultTimeout;
        TimeSpan observe = TimeSpan.Zero;
        string? screenshotPath = null;
        string? server = null;
        string? userName = null;
        string? remoteApplicationProgram = null;
        string? remoteApplicationArgs = null;
        var railLaunches = new List<string>();
        string passwordVariable = DefaultPasswordVariable;
        int desktopWidth = 1024;
        int desktopHeight = 768;

        for (int index = 1; index < args.Length; index++)
        {
            string argument = args[index];
            if (!operationSpecified && !classIdSpecified && Guid.TryParse(argument, out Guid parsedClassId))
            {
                classId = parsedClassId;
                classIdSpecified = true;
                continue;
            }

            if (!operationSpecified && argument is "probe" or "connect" or "unload")
            {
                operation = argument switch
                {
                    "probe" => Operation.Probe,
                    "connect" => Operation.Connect,
                    "unload" => Operation.Unload,
                    _ => throw new ArgumentException(),
                };
                operationSpecified = true;
                continue;
            }

            if (argument == "--connect" && !operationSpecified)
            {
                operation = Operation.Connect;
                operationSpecified = true;
                continue;
            }

            if (argument == "--show" && !show)
            {
                show = true;
                continue;
            }

            if (argument == "--autologon" && !autoLogon)
            {
                autoLogon = true;
                continue;
            }

            if ((argument is "--remoteapp" or "--remoteapp-program") && !remoteApplicationProgramSpecified)
            {
                remoteApplicationProgram = NextValue(args, ref index, argument);
                remoteApplicationProgramSpecified = true;
                continue;
            }

            if (argument == "--remoteapp-args" && !remoteApplicationArgsSpecified)
            {
                remoteApplicationArgs = NextValue(args, ref index, "--remoteapp-args", allowOptionLikeValue: true);
                remoteApplicationArgsSpecified = true;
                continue;
            }

            if (argument == "--rail-launch")
            {
                railLaunches.Add(NextValue(args, ref index, "--rail-launch"));
                continue;
            }

            if (argument == "--json" && !json)
            {
                json = true;
                continue;
            }

            if (argument == "--timeout" && !timeoutSpecified)
            {
                timeout = ParseSeconds(args, ref index, "--timeout");
                timeoutSpecified = true;
                continue;
            }

            if (argument == "--observe" && !observeSpecified)
            {
                observe = ParseSeconds(args, ref index, "--observe");
                observeSpecified = true;
                continue;
            }

            if (argument == "--screenshot" && !screenshotSpecified)
            {
                screenshotPath = NextValue(args, ref index, "--screenshot");
                screenshotSpecified = true;
                continue;
            }

            if (argument == "--server" && !serverSpecified)
            {
                server = NextValue(args, ref index, "--server");
                serverSpecified = true;
                continue;
            }

            if (argument == "--username" && !userNameSpecified)
            {
                userName = NextValue(args, ref index, "--username");
                userNameSpecified = true;
                continue;
            }

            if (argument == "--password-env" && !passwordVariableSpecified)
            {
                passwordVariable = NextValue(args, ref index, "--password-env");
                passwordVariableSpecified = true;
                continue;
            }

            if (argument == "--desktop-width" && !desktopWidthSpecified)
            {
                desktopWidth = ParseDesktopDimension(args, ref index, "--desktop-width");
                desktopWidthSpecified = true;
                continue;
            }

            if (argument == "--desktop-height" && !desktopHeightSpecified)
            {
                desktopHeight = ParseDesktopDimension(args, ref index, "--desktop-height");
                desktopHeightSpecified = true;
                continue;
            }

            throw new ArgumentException();
        }

        if ((operation is Operation.Probe or Operation.Unload)
            && (server is not null
                || userName is not null
                || remoteApplicationProgram is not null
                || remoteApplicationArgs is not null
                || railLaunches.Count != 0
                || screenshotPath is not null
                || passwordVariable != DefaultPasswordVariable
                || desktopWidthSpecified
                || desktopHeightSpecified))
        {
            throw new ArgumentException();
        }
        if (remoteApplicationArgs is not null && remoteApplicationProgram is null)
            throw new ArgumentException();

        if (show && observe == TimeSpan.Zero)
            observe = TimeSpan.FromSeconds(30);
        if (railLaunches.Count != 0 && observe == TimeSpan.Zero)
            throw new ArgumentException();
        if ((operation is Operation.Probe or Operation.Unload) && observe != TimeSpan.Zero && !show)
            throw new ArgumentException();

        return new AxHostOptions(
            libraryPath,
            classId,
            operation,
            autoLogon,
            remoteApplicationProgram,
            remoteApplicationArgs,
            railLaunches,
            show,
            json,
            timeout,
            observe,
            screenshotPath,
            server,
            userName,
            passwordVariable,
            desktopWidth,
            desktopHeight);
    }

    private static TimeSpan ParseSeconds(string[] args, ref int index, string option)
    {
        if (!int.TryParse(NextValue(args, ref index, option), out int seconds) || seconds is < 1 or > 600)
            throw new ArgumentException();

        return TimeSpan.FromSeconds(seconds);
    }

    private static int ParseDesktopDimension(string[] args, ref int index, string option)
    {
        if (!int.TryParse(NextValue(args, ref index, option), out int dimension) || dimension is < 200 or > 8192)
            throw new ArgumentException();

        return dimension;
    }

    private static string NextValue(string[] args, ref int index, string option, bool allowOptionLikeValue = false)
    {
        if (++index >= args.Length
            || string.IsNullOrWhiteSpace(args[index])
            || (!allowOptionLikeValue && args[index].StartsWith("-", StringComparison.Ordinal)))
            throw new ArgumentException($"{option} requires a value");

        return args[index];
    }

    private static void PrintUsage() =>
        Console.Error.WriteLine(
            "Usage: ironrdp-axhost <com-server.dll> [class-id] [probe|connect|unload] [--autologon] [--remoteapp-program <program>] [--remoteapp-args <arguments>] [--rail-launch <program>]... [--desktop-width <pixels>] [--desktop-height <pixels>] [--json] [--show] [--timeout <seconds>] [--observe <seconds>] [--server <host[:port]>] [--username <name>] [--password-env <variable>] [--screenshot <path>]\nUse --help-agent for the machine-readable contract.");

    private const string AgentGuide = """
# ironrdp-axhost

`ironrdp-axhost` is a one-shot Windows WinForms/AxHost end-to-end verifier for an IronRDP-compatible
COM server DLL. It loads the supplied DLL through `DllGetClassObject`, so it never changes global COM
registration. Every invocation exits after producing one result.

## Output contract

Pass `--json` to receive exactly one JSON object on stdout. It has `operation`, `passed`,
`exitCode`, `durationMilliseconds`, `hostHandle`, `connectedProperty`, `events`, `screenshot`,
`failure`, and `unloadStatus` fields. Event names and failure strings are bounded local diagnostics;
credentials, server addresses, remote errors, and packet data are never written.

Exit `0` means the requested assertions passed. Exit `1` means activation or lifecycle validation
failed. Exit `64` means the command line was invalid.

## Operations

- `probe` (default): instantiate the requested class, host it through WinForms `AxHost`, and retrieve
  the Automation object. The JSON `events` array contains `activated` on success.
- `unload`: prove teardown compatibility without connecting: instantiate through `AxHost`, advise and
  unadvise `IMsTscAxEvents`, release the connection point, let AxHost release the root control,
  then require
  `DllCanUnloadNow` to return `S_OK` before freeing the DLL. The JSON `unloadStatus` is that HRESULT.
- `connect`: run the complete lifecycle: configure the control, subscribe to
  `IMsTscAxEvents`, invoke `Connect`, require the `Connected` property or `OnConnected`, then
  disconnect and unadvise the sink. `events` records lifecycle and RemoteApp dispatch diagnostics.

Pass `--autologon` with `connect` to set the control's documented `IronRdpAutoLogon` extended
setting before the connection begins.
Pass `--remoteapp-program PROGRAM` with `connect` to enable RemoteApp and launch one program.
`--remoteapp` remains an alias for compatibility.
Pass `--remoteapp-args ARGUMENTS` with `connect` and `--remoteapp-program PROGRAM` to set the
program's optional arguments.

`--remoteapp-args <arguments>` accepts values beginning with `-`.
The preconnect options configure private IronRDP extended settings through the activated control,
so they do not require MSTSCLib registration or a system `mstscax.dll`.

Pass one or more `--rail-launch <program>` options to exercise subsequent RemoteApp execute requests
over the established RAIL channel. The host invokes the standard `ITSRemoteProgram::ServerStartProgram`
control interface after it observes a connected state, emits one `rail-launch-dispatched` event per
request, and requires an observation interval so the remote windows can be inspected.

`--desktop-width` and `--desktop-height` set the RDP desktop dimensions (each 200-8192 pixels).
Use a size that covers the RemoteApp server's monitor layout; the RAIL window coordinates and the
retained graphics frame share this coordinate space.

## Credentials and configuration

`connect` obtains values from process-local environment variables by default:

- `RDP_HOSTNAME`
- `RDP_USERNAME`
- `RDP_PASSWORD`

Use `--server` or `--username` to override the two non-secret values. Use
`--password-env NAME` to select a different password environment variable. Passwords are never
accepted on the command line, printed, or serialized into the JSON result.

## Evidence

`--screenshot PATH` (connect only) saves the AxHost rendering surface after a connected state is
observed. A screenshot failure fails the invocation. `--observe SECONDS` keeps the verified session
open for a bounded period before clean shutdown, and `--show` makes that window visible. `--show`
defaults `--observe` to 30 seconds. With a RemoteApp configuration, AxHost hides its otherwise
unused desktop canvas after connecting; the independently projected RAIL windows remain visible.
`--timeout SECONDS` bounds the wait for connection (default 30, range 1-600).

## Examples

```powershell
ironrdp-axhost .\target\release\ironrdpax.dll probe --json
ironrdp-axhost .\target\release\ironrdpax.dll connect --timeout 60 --screenshot .\artifacts\frame.png --json
ironrdp-axhost .\target\release\ironrdpax.dll connect --remoteapp-program '||notepad' --show --observe 30 --json
ironrdp-axhost <path-to-MsRdpEx.dll> {7CACBD7B-0D99-468F-AC33-22E495C0AFE5} connect --json
```
""";
}
