using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct CredsspProcessGenerator
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspProcessGenerator_start", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultGeneratorStateIronRdpError Start(CredsspProcessGenerator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspProcessGenerator_resume", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultGeneratorStateIronRdpError Resume(CredsspProcessGenerator* handle, DiplomatSliceU8 response);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CredsspProcessGenerator_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(CredsspProcessGenerator* handle);
}