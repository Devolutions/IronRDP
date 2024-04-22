using Avalonia;
using System;
using System.Diagnostics;

namespace Devolutions.IronRdp.AvaloniaExample;

class Program
{
    // Initialization code. Don't use any Avalonia, third-party APIs or any
    // SynchronizationContext-reliant code before AppMain is called: things aren't initialized
    // yet and stuff might break.
    [STAThread]
    public static void Main(string[] args)
    {
        InitializeLogging();
        BuildAvaloniaApp()
        .StartWithClassicDesktopLifetime(args);
    }

    // Avalonia configuration, don't remove; also used by visual designer.
    public static AppBuilder BuildAvaloniaApp()
        => AppBuilder.Configure<App>()
            .UsePlatformDetect()
            .WithInterFont()
            .LogToTrace();

    public static void InitializeLogging()
    {
        Trace.AutoFlush = true;
        TextWriterTraceListener myListener = new TextWriterTraceListener(System.IO.File.CreateText("AvaloniaExample.log"));
        Trace.Listeners.Add(myListener);
    }

}
