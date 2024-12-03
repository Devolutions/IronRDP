namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_SESSION_LOGOFF_MSG is used to request a user session logoff.
    ///
    /// NOW_PROTO: NOW_SESSION_LOGOFF_MSG
    /// </summary>
    public class NowMsgSessionLogoff : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassSession;
        public static byte TypeMessageKind => 0x02; // NOW-PROTO: NOW_SESSION_LOGOFF_MSG_ID

        public byte MessageClass => NowMessage.ClassSession;
        public byte MessageKind => 0x02;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 0;

        public void SerializeBody(NowWriteCursor cursor) { }
    }
}
