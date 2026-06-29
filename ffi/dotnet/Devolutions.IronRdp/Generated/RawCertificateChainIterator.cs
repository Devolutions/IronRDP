using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct CertificateChainIterator
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CertificateChainIterator_next", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern VecU8* Next(CertificateChainIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CertificateChainIterator_len", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern nuint Len(CertificateChainIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CertificateChainIterator_is_empty", CallingConvention = CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal static unsafe extern bool IsEmpty(CertificateChainIterator* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "CertificateChainIterator_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(CertificateChainIterator* handle);
}