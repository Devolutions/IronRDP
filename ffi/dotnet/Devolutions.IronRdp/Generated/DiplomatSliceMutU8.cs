using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Diplomat;

[StructLayout(LayoutKind.Sequential)]
internal unsafe struct DiplomatSliceMutU8
{
    public byte* Ptr;
    public nuint Len;
}