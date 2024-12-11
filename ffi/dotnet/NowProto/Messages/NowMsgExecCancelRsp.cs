using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_RESULT_MSG message is used to return the result of an execution request.
    ///
    /// NOW_PROTO: NOW_EXEC_RESULT_MSG
    /// </summary>
    public class NowMsgExecCancelRsp(uint sessionId, NowStatus status) : INowDeserialize<NowMsgExecCancelRsp>
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x03; // NOW-PROTO: NOW_EXEC_RESULT_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x03;

        // -- INowDeserialize --

        public static NowMsgExecCancelRsp Deserialize(ushort flags, NowReadCursor cursor)
        {
            var sessionId = cursor.ReadUInt32Le();
            var status = NowStatus.Deserialize(cursor);

            return new NowMsgExecCancelRsp(sessionId, status);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
        public NowStatus Status { get; set; } = status;
    }
}
