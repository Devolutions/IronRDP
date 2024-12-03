namespace NowProto.Types
{
    /// <summary>
    /// A status code, with a structure similar to HRESULT.
    ///
    /// NOW-PROTO: NOW_STATUS
    /// </summary>
    public class NowStatus(NowStatus.StatusSeverity severity, byte kind, ushort status)
    {
        internal const uint FixedPartSize = 4; /* u8 severity + u8 kind + u16 status code */

        public enum StatusSeverity : byte
        {
            /// <summary>
            /// Informative status
            ///
            /// NOW-PROTO: NOW_SEVERITY_INFO
            /// </summary>
            Info = 0,
            /// <summary>
            /// Warning status
            ///
            /// NOW-PROTO: NOW_SEVERITY_WARN
            /// </summary>
            Warn = 1,
            /// <summary>
            /// Error status (recoverable)
            ///
            /// NOW-PROTO: NOW_SEVERITY_ERROR
            /// </summary>
            Error = 2,
            /// <summary>
            /// Error status (non-recoverable)
            ///
            /// NOW-PROTO: NOW_SEVERITY_FATAL
            /// </summary>
            Fatal = 3,
        }

        public enum KnownStatusCode : ushort
        {
            /// <summary>
            /// NOW-PROTO: NOW_CODE_SUCCESS
            /// </summary>
            Success = 0x0000,
            /// <summary>
            /// NOW-PROTO: NOW_CODE_FAILURE
            /// </summary>
            Failure = 0xFFFF,
            /// <summary>
            /// NOW-PROTO: NOW_CODE_FILE_NOT_FOUND
            /// </summary>
            FileNotFound = 0x0002,
            /// <summary>
            /// NOW-PROTO: NOW_CODE_ACCESS_DENIED
            /// </summary>
            AccessDenied = 0x0005,
            /// <summary>
            /// NOW-PROTO: NOW_CODE_BAD_FORMAT
            /// </summary>
            BadFormat = 0x000B,
        }

        public static NowStatus Deserialize(NowReadCursor cursor)
        {
            cursor.EnsureEnoughBytes(FixedPartSize);
            var header = cursor.ReadByte();
            var severity = (StatusSeverity)(header >> 6);
            var kind = cursor.ReadByte();
            var status = cursor.ReadUInt16Le();

            return new NowStatus(severity, kind, status);
        }

        public void Serialize(NowWriteCursor cursor)
        {
            cursor.EnsureEnoughBytes(FixedPartSize);
            cursor.WriteByte((byte)((byte)Severity << 6));
            cursor.WriteByte(Kind);
            cursor.WriteUint16Le(Status);
        }


        public KnownStatusCode? KnownStatus
        {
            get
            {
                if (Enum.IsDefined(typeof(KnownStatusCode), Status))
                {
                    return (KnownStatusCode)Status;
                }

                return null;
            }
        }

        public StatusSeverity Severity = severity;
        public byte Kind = kind;
        public ushort Status = status;
    }
}
