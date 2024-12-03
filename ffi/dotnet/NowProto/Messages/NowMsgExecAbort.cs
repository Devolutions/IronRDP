using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_ABORT_MSG message is used to abort a remote execution immediately due to an
    /// unrecoverable error. This message can be sent at any time without an explicit response message.
    /// The session is considered aborted as soon as this message is sent.
    ///
    /// NOW-PROTO: NOW_EXEC_ABORT_MSG
    /// </summary>
    public class NowMsgExecAbort(uint sessionId, NowStatus status) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x01; // NOW-PROTO: NOW_EXEC_ABORT_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x01;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 4 + NowStatus.FixedPartSize;

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
            Status.Serialize(cursor);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
        public NowStatus Status { get; set; } = status;
    }
}
