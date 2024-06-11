namespace Devolutions.IronRdp;

public interface ISequence
{
    PduHint? NextPduHint();
    Written Step(byte[] pduHint, WriteBuf buf);
    Written StepNoInput(WriteBuf buf);
}

