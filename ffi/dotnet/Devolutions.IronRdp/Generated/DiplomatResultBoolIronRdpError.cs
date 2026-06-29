using System;
using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Raw;

using Devolutions.IronRdp;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DiplomatResultBoolIronRdpError
{
    [StructLayout(LayoutKind.Explicit)]
    private unsafe struct InnerUnion
    {
        [FieldOffset(0)] internal byte ok;
        [FieldOffset(0)] internal IronRdpError* err;
    }

    private InnerUnion _inner;

    [MarshalAs(UnmanagedType.U1)]
    public bool IsOk;
    public bool Ok => IsOk ? _inner.ok != 0 : throw new InvalidOperationException("Result does not contain Ok value");
    public unsafe IronRdpError* Err => !IsOk ? _inner.err : throw new InvalidOperationException("Result does not contain Err value");
}