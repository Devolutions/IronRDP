namespace Devolutions.IronRdp;

public partial class ClientConnector : ISequence
{
    public Written Step(byte[] pdu, MonotonicInstant receivedAt, WriteBuf buf)
    {
        return Step(pdu, receivedAt.Milliseconds, buf);
    }
}
