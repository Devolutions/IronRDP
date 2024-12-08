namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_CANCEL_REQ_MSG message is used to cancel a remote execution session.
    ///
    /// NOW-PROTO: NOW_EXEC_CANCEL_REQ_MSG
    /// </summary>
    public class NowMsgExecCancelReq(uint sessionId) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x02; // NOW-PROTO: NOW_EXEC_CANCEL_REQ_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x02;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 4 /* u32 SessionId */;

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
    }
}
