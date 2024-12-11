using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_PWSH_MSG message is used to execute a remote Windows PowerShell (powershell.exe) command.
    ///
    /// NOW-PROTO: NOW_EXEC_PWSH_MSG
    /// </summary>
    public class NowMsgExecPwsh(uint sessionId, string command) : INowSerialize
    {
        // -- INowMessage --
        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x16; // NOW-PROTO: NOW_EXEC_PWSH_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x16;

        // -- INowSerialize --

        public ushort Flags { get; private set; }
        
        public uint BodySize => 4 /* u32 sessionId */
                                + NowVarStr.LengthOf(Command)
                                + NowVarStr.LengthOf(_executionPolicy)
                                + NowVarStr.LengthOf(_configurationName);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
            cursor.WriteVarStr(Command);
            cursor.WriteVarStr(_executionPolicy);
            cursor.WriteVarStr(_configurationName);
        }

        // -- impl --

        private const ushort FlagNoLogo = 0x0001;
        private const ushort FlagNoExit = 0x0002;
        private const ushort FlagSta = 0x0004;
        private const ushort FlagMta = 0x0008;
        private const ushort FlagNoProfile = 0x0010;
        private const ushort FlagNonInteractive = 0x0020;
        private const ushort FlagExecutionPolicy = 0x0040;
        private const ushort FlagConfigurationName = 0x0080;

        public enum ApartmentStateKind : ushort
        {
            Sta = FlagSta,
            Mta = FlagMta,
        }

        /// <summary>
        /// PowerShell -NoLogo option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_LOGO
        /// </summary>
        public bool NoLogo
        {
            get => (Flags & FlagNoLogo) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagNoLogo;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagNoLogo);
                }
            }
        }

        /// <summary>
        /// PowerShell -NoExit option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_EXIT
        /// </summary>
        public bool NoExit
        {
            get => (Flags & FlagNoExit) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagNoExit;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagNoExit);
                }
            }
        }

        /// <summary>
        /// PowerShell -Mta & -Sta options
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_MTA/NOW_EXEC_FLAG_PS_STA
        /// </summary>
        public ApartmentStateKind? ApartmentState
        {
            get
            {
                var sta = (Flags & FlagSta) != 0;
                var mta = (Flags & FlagMta) != 0;
                if (sta && mta)
                {
                    throw new NowProtoException(NowProtoException.Kind.InvalidApartmentStateFlags);
                }

                if (!(sta || mta))
                {
                    // Not specified
                    return null;
                }

                return sta ? ApartmentStateKind.Sta : ApartmentStateKind.Mta;
            }
            set
            {
                Flags &= unchecked((ushort)~(FlagSta | FlagMta));
                if (value == null)
                {
                    return;
                }

                Flags |= (ushort)value;
            }
        }

        /// <summary>
        /// PowerShell -NoProfile option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_PROFILE
        /// </summary>
        public bool NoProfile
        {
            get => (Flags & FlagNoProfile) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagNoProfile;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagNoProfile);
                }
            }
        }

        /// <summary>
        /// PowerShell -NonInteractive option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NON_INTERACTIVE
        /// </summary>
        public bool NonInteractive
        {
            get => (Flags & FlagNonInteractive) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagNonInteractive;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagNonInteractive);
                }
            }
        }

        /// <summary>
        /// The PowerShell -ExecutionPolicy parameter.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_EXECUTION_POLICY
        /// </summary>
        public string? ExecutionPolicy
        {
            get => (Flags & FlagExecutionPolicy) == 0 ? null : _executionPolicy!;
            set
            {
                if (value == null)
                {
                    Flags &= unchecked((ushort)~FlagExecutionPolicy);
                    _executionPolicy = "";
                    return;
                }

                Flags |= FlagExecutionPolicy;
                _executionPolicy = value;
            }
        }


        /// <summary>
        /// The PowerShell -ConfigurationName parameter.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_CONFIGURATION_NAME
        /// </summary>
        public string? ConfigurationName
        {
            get => (Flags & FlagConfigurationName) == 0 ? null : _configurationName!;
            set
            {
                if (value == null)
                {
                    Flags &= unchecked((ushort)~FlagConfigurationName);
                    _configurationName = "";
                    return;
                }

                Flags |= FlagConfigurationName;
                _configurationName = value;
            }
        }

        public uint SessionId { get; set; } = sessionId;
        public string Command { get; set; } = command;

        private string _executionPolicy = "";
        private string _configurationName = "";
    }
}
