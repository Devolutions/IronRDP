namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_CAPSET_MSG message is sent to advertise capabilities.
    ///
    /// NOW-PROTO: NOW_EXEC_CAPSET_MSG
    /// </summary>
    public class NowMsgExecCaps: INowSerialize, INowDeserialize<NowMsgExecCaps>
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x00; // NOW-PROTO: NOW_EXEC_CAPSET_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x00;

        // -- INowDeserialize --

        public static NowMsgExecCaps Deserialize(ushort flags, NowReadCursor cursor)
        {
            return new NowMsgExecCaps()
            {
                Flags = flags
            };
        }

        // -- INowSerialize --

        public ushort Flags { get; private set; }
        public uint BodySize => 0;

        public void SerializeBody(NowWriteCursor cursor) { }

        // -- impl --

        private const ushort FlagRun = 0x0001;
        private const ushort FlagCmd = 0x0002;
        private const ushort FlagProcess = 0x0004;
        private const ushort FlagShell = 0x0008;
        private const ushort FlagBatch = 0x0010;
        private const ushort FlagWinPs = 0x0020;
        private const ushort FlagPwsh = 0x0040;
        private const ushort FlagAppleScript = 0x0080;


        /// <summary>
        /// Generic "Run" execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_RUN
        /// </summary>
        public bool CapabilityRun
        {
            get => (Flags & FlagRun) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagRun;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagRun);
                }
            }
        }

        /// <summary>
        /// Generic command execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_CMD
        /// </summary>
        public bool CapabilityCmd
        {
            get => (Flags & FlagCmd) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagCmd;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagCmd);
                }
            }
        }

        /// <summary>
        /// CreateProcess() execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_PROCESS
        /// </summary>
        public bool CapabilityProcess
        {
            get => (Flags & FlagProcess) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagProcess;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagProcess);
                }
            }
        }

        /// <summary>
        /// System shell (.sh) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_SHELL
        /// </summary>
        public bool CapabilityShell
        {
            get => (Flags & FlagShell) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagShell;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagShell);
                }
            }
        }

        /// <summary>
        /// Windows batch file (.bat) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_BATCH
        /// </summary>
        public bool CapabilityBatch
        {
            get => (Flags & FlagBatch) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagBatch;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagBatch);
                }
            }
        }

        /// <summary>
        /// Windows PowerShell (.ps1) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_WINPS
        /// </summary>
        public bool CapabilityWinPs
        {
            get => (Flags & FlagWinPs) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagWinPs;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagWinPs);
                }
            }
        }

        /// <summary>
        /// PowerShell 7 (.ps1) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_PWSH
        /// </summary>
        public bool CapabilityPwsh
        {
            get => (Flags & FlagPwsh) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagPwsh;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagPwsh);
                }
            }
        }

        /// <summary>
        /// Applescript (.scpt) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_APPLESCRIPT
        /// </summary>
        public bool CapabilityAppleScript
        {
            get => (Flags & FlagAppleScript) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagAppleScript;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagAppleScript);
                }
            }
        }
    }
}
