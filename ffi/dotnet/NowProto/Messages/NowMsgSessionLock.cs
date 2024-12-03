namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_SESSION_LOCK_MSG is used to request locking the user session.
    ///
    /// NOW_PROTO: NOW_SESSION_LOCK_MSG
    /// </summary>
    public class NowMsgSessionLock : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassSession;
        public static byte TypeMessageKind => 0x01; // NOW-PROTO: NOW_SESSION_LOCK_MSG_ID

        public byte MessageClass => NowMessage.ClassSession;
        public byte MessageKind => 0x01;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 0;

        public void SerializeBody(NowWriteCursor cursor) { }
    }
}
