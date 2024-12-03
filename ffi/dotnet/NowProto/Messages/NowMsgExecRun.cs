using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_RUN_MSG message is used to send a run request. This request type maps to starting
    /// a program by using the “Run” menu on operating systems (the Start Menu on Windows, the Dock on
    /// macOS etc.). The execution of programs started with NOW_EXEC_RUN_MSG is not followed and does
    /// not send back the output.
    ///
    /// NOW_PROTO: NOW_EXEC_RUN_MSG
    /// </summary>
    public class NowMsgExecRun(uint sessionId, string command) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x10; // NOW-PROTO: NOW_EXEC_RUN_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x10;

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