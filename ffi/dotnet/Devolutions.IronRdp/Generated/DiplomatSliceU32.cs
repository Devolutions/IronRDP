using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Diplomat;

[StructLayout(LayoutKind.Sequential)]
internal unsafe struct DiplomatSliceU32
{
    public uint* Ptr;
    public nuint Len;
}