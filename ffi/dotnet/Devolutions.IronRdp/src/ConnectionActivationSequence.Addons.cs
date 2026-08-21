namespace Devolutions.IronRdp;

public partial class ConnectionActivationSequence : ISequence
{
    public Written Step(byte[] pdu, MonotonicInstant receivedAt, WriteBuf buf)
    {
        return Step(pdu, receivedAt.Milliseconds, buf);
    }
}
