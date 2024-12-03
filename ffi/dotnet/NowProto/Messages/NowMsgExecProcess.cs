using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_PROCESS_MSG message is used to send a Windows CreateProcess() request.
    ///
    /// NOW-PROTO: NOW_EXEC_PROCESS_MSG
    /// </summary>
    public class NowMsgExecProcess(uint sessionId, string filename) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x12; // NOW-PROTO: NOW_EXEC_PROCESS_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x12;

        // -- INowSerialize --

        public ushort Flags => 0;
        public uint BodySize => 4 /* u32 SessionId */
                                + NowVarStr.LengthOf(Filename)
                                + NowVarStr.LengthOf(Parameters)
                                + NowVarStr.LengthOf(Directory);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
            cursor.WriteVarStr(Filename);
            cursor.WriteVarStr(Parameters);
            cursor.WriteVarStr(Directory);
        }

        // -- impl --

        public uint SessionId { get; set; } = sessionId;
        public string Filename { get; set; } = filename;
        public string Parameters { get; set; } = "";
        public string Directory { get; set; } = "";
    }
}