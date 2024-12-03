using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_SYSTEM_SHUTDOWN_MSG structure is used to request a system shutdown.
    ///
    /// NOW_PROTO: NOW_SYSTEM_SHUTDOWN_MSG
    /// </summary>
    public class NowMsgSystemShutdown(string message, uint timeout) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassSystem;
        public static byte TypeMessageKind => 0x03; // NOW-PROTO: NOW_SYSTEM_SHUTDOWN_ID

        public byte MessageClass => NowMessage.ClassSystem;
        public byte MessageKind => 0x03;

        // -- INowSerialize --

        public ushort Flags { get; private set; }
        public uint BodySize => 4 /* u32 timeout */ + NowVarStr.LengthOf(Message);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(Timeout);
            cursor.WriteVarStr(Message);
        }

        // -- impl --

        private const ushort FlagForce = 0x0001;
        private const ushort FlagReboot = 0x0002;

        /// <summary>
        /// Force shutdown
        ///
        /// NOW-PROTO: NOW_SHUTDOWN_FLAG_FORCE
        /// </summary>
        public bool Force
        {
            get => (Flags & FlagForce) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagForce;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagForce);
                }
            }
        }

        /// <summary>
        /// Reboot after shutdown
        ///
        /// NOW-PROTO: NOW_SHUTDOWN_FLAG_REBOOT
        /// </summary>
        public bool Reboot
        {
            get => (Flags & FlagReboot) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagReboot;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagReboot);
                }
            }
        }

        public uint Timeout { get; set; } = timeout;
        public string Message { get; set; } = message;
    }
}
