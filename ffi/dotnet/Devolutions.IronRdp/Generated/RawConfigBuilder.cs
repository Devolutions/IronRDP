using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConfigBuilder
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_new", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ConfigBuilder* New();

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_with_username_and_password", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void WithUsernameAndPassword(ConfigBuilder* handle, DiplomatSliceU8 username, DiplomatSliceU8 password);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_domain", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetDomain(ConfigBuilder* handle, DiplomatSliceU8 domain);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_enable_tls", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetEnableTls(ConfigBuilder* handle, [MarshalAs(UnmanagedType.U1)] bool enableTls);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_enable_credssp", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetEnableCredssp(ConfigBuilder* handle, [MarshalAs(UnmanagedType.U1)] bool enableCredssp);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_keyboard_layout", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetKeyboardLayout(ConfigBuilder* handle, uint keyboardLayout);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_keyboard_type", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetKeyboardType(ConfigBuilder* handle, KeyboardType keyboardType);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_keyboard_subtype", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetKeyboardSubtype(ConfigBuilder* handle, uint keyboardSubtype);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_keyboard_functional_keys_count", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetKeyboardFunctionalKeysCount(ConfigBuilder* handle, uint keyboardFunctionalKeysCount);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_ime_file_name", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetImeFileName(ConfigBuilder* handle, DiplomatSliceU8 imeFileName);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_dig_product_id", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetDigProductId(ConfigBuilder* handle, DiplomatSliceU8 digProductId);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_desktop_size", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetDesktopSize(ConfigBuilder* handle, ushort height, ushort width);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_performance_flags", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetPerformanceFlags(ConfigBuilder* handle, PerformanceFlags* performanceFlags);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_bitmap_config", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetBitmapConfig(ConfigBuilder* handle, BitmapConfig* bitmap);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_client_build", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetClientBuild(ConfigBuilder* handle, uint clientBuild);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_client_name", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetClientName(ConfigBuilder* handle, DiplomatSliceU8 clientName);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_client_dir", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetClientDir(ConfigBuilder* handle, DiplomatSliceU8 clientDir);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_enable_server_pointer", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetEnableServerPointer(ConfigBuilder* handle, [MarshalAs(UnmanagedType.U1)] bool enableServerPointer);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_autologon", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetAutologon(ConfigBuilder* handle, [MarshalAs(UnmanagedType.U1)] bool autologon);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_pointer_software_rendering", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetPointerSoftwareRendering(ConfigBuilder* handle, [MarshalAs(UnmanagedType.U1)] bool pointerSoftwareRendering);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_set_dvc_pipe_proxy", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern void SetDvcPipeProxy(ConfigBuilder* handle, DvcPipeProxyConfig* dvcPipeProxy);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_build", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultConfigIronRdpError Build(ConfigBuilder* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConfigBuilder_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConfigBuilder* handle);
}