using System;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Devolutions.IronRdp;

// iOS is the only target that ships the native code inside a framework bundle instead of a loose dylib, and
// no default probing permutation resolves a path inside one. This only bites where P/Invoke binds at run time
// (the simulator's JIT, and the interpreter) — a full AOT build emits direct calls and never probes.
internal static class NativeLibraryResolver
{
    // Matches the `NativeLib` constant the generated Raw* types pass to DllImport.
    private const string LibraryName = "DevolutionsIronRdp";

    private const string FrameworkBinary = "libDevolutionsIronRdp.framework/libDevolutionsIronRdp";

    private const string ProbeSymbol = "IronRdpError_get_kind";

    // CA2255 discourages module initializers in libraries, but the DllImports are spread across the generated
    // Raw* types with no shared static-constructor chokepoint, and requiring consumers to call an initializer
    // is the coupling this exists to avoid.
#pragma warning disable CA2255
    [ModuleInitializer]
#pragma warning restore CA2255
    internal static void Register()
    {
        if (!OperatingSystem.IsIOS())
        {
            return;
        }

        try
        {
            NativeLibrary.SetDllImportResolver(typeof(NativeLibraryResolver).Assembly, Resolve);
        }
        catch (InvalidOperationException)
        {
            // A host registered its own resolver first; theirs wins.
        }
    }

    private static IntPtr Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (!string.Equals(libraryName, LibraryName, StringComparison.Ordinal))
        {
            return IntPtr.Zero;
        }

        // The NativeReference links the framework, so its install name resolves through the app's @rpath.
        if (NativeLibrary.TryLoad($"@rpath/{FrameworkBinary}", out IntPtr handle)
            || NativeLibrary.TryLoad(Path.Combine(AppContext.BaseDirectory, "Frameworks", FrameworkBinary), out handle))
        {
            return handle;
        }

        // Statically linked or already loaded: the symbols are in the global namespace. Probing first keeps a
        // genuinely missing library reported as DllNotFoundException naming it, rather than as a per-symbol
        // EntryPointNotFoundException raised later from an unrelated call.
        IntPtr mainProgram = NativeLibrary.GetMainProgramHandle();

        return NativeLibrary.TryGetExport(mainProgram, ProbeSymbol, out _) ? mainProgram : IntPtr.Zero;
    }
}
