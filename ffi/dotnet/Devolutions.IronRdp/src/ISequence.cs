namespace Devolutions.IronRdp;

public interface ISequence
{
    PduHint? NextPduHint();

    /// <summary>
    /// Advances the sequence with a PDU that arrived at <paramref name="receivedAt"/>.
    /// </summary>
    /// <remarks>
    /// <paramref name="receivedAt"/> comes from the read that produced <paramref name="pdu"/>, so
    /// it must be the value <c>Framed</c> returned alongside the bytes, not a fresh reading taken
    /// at call time.
    /// </remarks>
    Written Step(byte[] pdu, MonotonicInstant receivedAt, WriteBuf buf);

    Written StepNoInput(WriteBuf buf);
}
