namespace NowProto
{
    /// <summary>
    /// Wrapper type for messages transferred over the NOW-PROTO communication channel.
    ///
    /// NOW-PROTO: NOW_*_MSG messages
    /// </summary>
    public class NowMessage
    {
        // NOW-PROTO: NOW_SYSTEM_MSG_CLASS_ID
        internal const byte ClassSystem = 0x11;

        // NOW-PROTO: NOW_SESSION_MSG_CLASS_ID
        internal const byte ClassSession = 0x12;

        // NOW-PROTO: NOW_EXEC_MSG_CLASS_ID
        internal const byte ClassExec = 0x13;

        /// <summary>
        /// Immutable message view. Allows to read message header and body without deserialization
        /// or to perform deserialization to specific message type on demand.
        /// </summary>
        public readonly struct NowMessageView
        {
            internal NowMessageView(NowHeader header, ArraySegment<byte> body)
            {
                _header = header;
                _body = body;
            }

            /// <summary>
            /// Converts stack allocated message view to owned reference-counted message.
            /// </summary>
            public NowMessage ToOwned()
            {
                var bodyCopy = new byte[_body.Count];
                _body.CopyTo(bodyCopy);
                return new NowMessage(_header, bodyCopy);
            }

            /// <summary>
            /// Deserializes message to a specific message type.
            /// </summary>
            public T Deserialize<T>() where T : INowDeserialize<T>
            {
                if (_header.MsgClass != T.TypeMessageClass)
                {
                    throw new NowProtoException(NowProtoException.Kind.DifferentMessageClass);
                }

                if (_header.MsgKind != T.TypeMessageKind)
                {
                    throw new NowProtoException(NowProtoException.Kind.DifferentMessageKind);
                }

                var reader = new NowReadCursor(_body);
                return T.Deserialize(_header.Flags, reader);
            }

            public byte MessageClass => _header.MsgClass;
            public byte MessageKind => _header.MsgKind;
            public ushort Flags => _header.Flags;
            public ReadOnlySpan<byte> Body => _body;

            private readonly NowHeader _header;
            private readonly ArraySegment<byte> _body;
        }

        /// <summary>
        /// Constructs new arbitrary message. Underlying body/class/kind correctness is not validated.
        /// </summary>
        public NowMessage(byte msgClass, byte msgKind, ushort flags, byte[] body)
        {
            _header = new NowHeader
            {
                Size = (uint)body.Length,
                MsgClass = msgClass,
                MsgKind = msgKind,
                Flags = flags,
            };

            _body = body;
        }

        private NowMessage(NowHeader header, byte[] body)
        {
            _header = header;
            _body = body;
        }

        /// <summary>
        /// Helper method to ensure enough bytes are available in the input span to read a full message
        /// prior to deserialization.
        /// </summary>
        public static bool IsInputHasEnoughBytes(ArraySegment<byte> input)
        {
            if (input.Count < NowHeader.FixedPartSize)
            {
                return false;
            }
            var header = NowHeader.Deserialize(new NowReadCursor(input));
            return input.Count >= NowHeader.FixedPartSize + header.Size;
        }

    /// <summary>
    /// Reads NowMessage from the input span represented as a `NowMessageView`.
    /// Note that the actual message body deserialization is not performed here.
    /// See `NowMessageView` methods for deserialization.
    /// </summary>
    public static NowMessageView Read(NowReadCursor cursor)
        {
            var header = NowHeader.Deserialize(cursor);
            var body = cursor.ReadBytes(header.Size);

            return new NowMessageView(header, body);
        }


        private NowHeader _header;
        private byte[] _body;
    }
}
