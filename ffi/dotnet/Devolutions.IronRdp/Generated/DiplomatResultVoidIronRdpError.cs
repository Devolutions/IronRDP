using System;
using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Raw;

using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DiplomatResultVoidIronRdpError
{
    [StructLayout(LayoutKind.Explicit)]
    private unsafe struct InnerUnion
    {
        [FieldOffset(0)] internal IronRdpError* err;
    }

    private InnerUnion _inner;

    public DiplomatBool IsOk;
    public unsafe IronRdpError* Err => !IsOk ? _inner.err : throw new InvalidOperationException("Result does not contain Err value");
}