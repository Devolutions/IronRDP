using System;
using System.Runtime.InteropServices;
using Devolutions.IronRdp;
using Devolutions.IronRdp.Diplomat;

namespace Devolutions.IronRdp.Raw;

[StructLayout(LayoutKind.Sequential)]
internal partial struct ConnectionActivationSequence
{

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationSequence_get_state", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern ConnectionActivationState* GetState(ConnectionActivationSequence* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationSequence_next_pdu_hint", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultPduHintIronRdpError NextPduHint(ConnectionActivationSequence* handle);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationSequence_step", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWrittenIronRdpError Step(ConnectionActivationSequence* handle, DiplomatSliceU8 pduHint, WriteBuf* buf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationSequence_step_no_input", CallingConvention = CallingConvention.Cdecl)]
internal static unsafe extern DiplomatResultWrittenIronRdpError StepNoInput(ConnectionActivationSequence* handle, WriteBuf* buf);

    [DllImport(DiplomatNativeLib.Name, EntryPoint = "ConnectionActivationSequence_destroy", CallingConvention = CallingConvention.Cdecl)]
    internal static unsafe extern void Destroy(ConnectionActivationSequence* handle);
}