namespace NowProto
{
    /// <summary>
    /// NowProto exception.
    /// </summary>
    public class NowProtoException: Exception
    {
        internal NowProtoException(Kind kind) : base(KindToString(kind))
        {
            ExceptionKind = kind;
        }

        public enum Kind
        {
            NotEnoughData,
            BufferTooSmall,
            DifferentMessageClass,
            DifferentMessageKind,
            VarU32OutOfRange,
            InvalidStatusSeverity,
            InvalidDataStreamFlags,
            InvalidApartmentStateFlags,
        }

        private static string KindToString(Kind kind)
        {
            return kind switch
            {
                Kind.NotEnoughData => "Not enough data",
                Kind.BufferTooSmall => "Buffer too small",
                Kind.DifferentMessageClass => "Different message class",
                Kind.DifferentMessageKind => "Different message kind",
                Kind.VarU32OutOfRange => "VarU32 out of range",
                Kind.InvalidStatusSeverity => "Invalid status severity",
                Kind.InvalidDataStreamFlags => "Invalid data stream flags",
                Kind.InvalidApartmentStateFlags => "Invalid apartment state flags",
                _ => "Unknown exception"
            };
        }

        public Kind ExceptionKind { get; }
    }
}
