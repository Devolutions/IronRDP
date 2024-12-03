using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_BATCH_MSG message is used to execute a remote batch command.
    ///
    /// NOW-PROTO: NOW_EXEC_BATCH_MSG
    /// </summary>
    public class NowMsgExecBatch(uint sessionId, string command) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x14; // NOW-PROTO: NOW_EXEC_BATCH_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x14;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 4 /* u32 SessionId */ + NowVarStr.LengthOf(Command);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
            cursor.WriteVarStr(Command);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
        public string Command { get; set; } = command;
    }
}