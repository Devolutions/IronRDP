using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct Config
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Config_get_builder", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ConfigBuilder* GetBuilder();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Config_get_dvc_pipe_proxy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DvcPipeProxyConfig* GetDvcPipeProxy(Config* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "Config_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(Config* handle);
}