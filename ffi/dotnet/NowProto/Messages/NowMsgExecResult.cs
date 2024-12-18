using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_RESULT_MSG message is used to return the result of an execution request.
    ///
    /// NOW_PROTO: NOW_EXEC_RESULT_MSG
    /// </summary>
    public class NowMsgExecResult(uint sessionId, NowStatus status) : INowDeserialize<NowMsgExecResult>
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x04; // NOW-PROTO: NOW_EXEC_RESULT_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x04;

        // -- INowDeserialize --

        public static NowMsgExecResult Deserialize(ushort flags, NowReadCursor cursor)
        {
            var sessionId = cursor.ReadUInt32Le();
            var status = NowStatus.Deserialize(cursor);

            return new NowMsgExecResult(sessionId, status);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
        public NowStatus Status { get; set; } = status;
    }
}