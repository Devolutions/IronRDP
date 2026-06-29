using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct InputDatabase
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InputDatabase_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern InputDatabase* New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InputDatabase_apply", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern FastPathInputEventIterator* Apply(InputDatabase* handle, Operation* operation);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "InputDatabase_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(InputDatabase* handle);
}