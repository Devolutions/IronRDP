using System;
using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Raw;

using Devolutions.IronRdp;

[StructLayout(LayoutKind.Sequential)]
internal partial struct DiplomatResultActiveStageOutputIteratorIronRdpError
{
    [StructLayout(LayoutKind.Explicit)]
    private unsafe struct InnerUnion
    {
        [FieldOffset(0)] internal ActiveStageOutputIterator* ok;
        [FieldOffset(0)] internal IronRdpError* err;
    }

    private InnerUnion _inner;

    [MarshalAs(UnmanagedType.U1)]
    public bool IsOk;
    public unsafe ActiveStageOutputIterator* Ok => IsOk ? _inner.ok : throw new InvalidOperationException("Result does not contain Ok value");
    public unsafe IronRdpError* Err => !IsOk ? _inner.err : throw new InvalidOperationException("Result does not contain Err value");
}