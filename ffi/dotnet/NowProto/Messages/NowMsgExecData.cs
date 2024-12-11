using NowProto.Types;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_EXEC_DATA_MSG message is used to send input/output data as part of a remote execution.
    ///
    /// NOW-PROTO: NOW_EXEC_DATA_MSG
    /// </summary>
    public class NowMsgExecData : INowSerialize, INowDeserialize<NowMsgExecData>
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassExec;
        public static byte TypeMessageKind => 0x05; // NOW-PROTO: NOW_EXEC_DATA_MSG_ID

        public byte MessageClass => NowMessage.ClassExec;
        public byte MessageKind => 0x05;

        // -- INowDeserialize --

        public static NowMsgExecData Deserialize(ushort flags, NowReadCursor cursor)
        {
            var sessionId = cursor.ReadUInt32Le();
            var data = cursor.ReadVarBuf();

            return new NowMsgExecData(sessionId, flags, data);
        }

        // -- INowSerialize --

        public ushort Flags { get; private set; }

        public uint BodySize => 4 /* u32 SessionId */ + NowVarBuf.LengthOf(Data);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.WriteUint32Le(SessionId);
            cursor.WriteVarBuf(Data);
        }

        // -- impl --

        public NowMsgExecData(uint sessionId, StreamKind stream, ArraySegment<byte> data)
            : this(sessionId, (ushort) stream, data) {}

        private NowMsgExecData(uint sessionId, ushort flags, ArraySegment<byte> data)
        {
            SessionId = sessionId;
            this.Flags = flags;
            Data = data;
        }

        private const ushort FlagFirst = 0x0001;
        private const ushort FlagLast = 0x0002;
        private const ushort FlagStdin = 0x0004;
        private const ushort FlagStdout = 0x0008;
        private const ushort FlagStderr = 0x0010;

        public enum StreamKind: ushort
        {
            Stdin = FlagStdin,
            Stdout = FlagStdout,
            Stderr = FlagStderr
        }

        /// <summary>
        /// Standard io stream kind.
        ///
        /// PROTO: NOW_EXEC_FLAG_DATA_STDIN, NOW_EXEC_FLAG_DATA_STDOUT, NOW_EXEC_FLAG_DATA_STDERR
        /// </summary>
        public StreamKind Stream
        {
            get
            {
                StreamKind? streamKind = null;
                var streamFlagsCount = 0;

                foreach (var kind in Enum.GetValues<StreamKind>())
                {
                    if ((Flags & (ushort)kind) == 0) continue;
                    streamKind = kind;
                    ++streamFlagsCount;
                }

                if (streamFlagsCount != 1)
                {
                    throw new NowProtoException(NowProtoException.Kind.InvalidDataStreamFlags);
                }

                return (StreamKind)streamKind!;
            }
            set
            {
                Flags &= unchecked((ushort)~(FlagStdin | FlagStdout | FlagStderr));
                Flags |= (ushort)value;
            }
        }

        /// <summary>
        /// This is the first data message.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_FIRST
        /// </summary>
        public bool Fist
        {
            get => (Flags & FlagFirst) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagFirst;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagFirst);
                }
            }
        }

        /// <summary>
        /// This is the last data message, the command completed execution.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_LAST
        /// </summary>
        public bool Last
        {
            get => (Flags & FlagLast) != 0;
            set
            {
                if (value)
                {
                    Flags |= FlagLast;
                }
                else
                {
                    Flags &= unchecked((ushort)~FlagLast);
                }
            }
        }

        public uint SessionId { get; set; }
        public ArraySegment<byte> Data { get; set; }
    }
}
