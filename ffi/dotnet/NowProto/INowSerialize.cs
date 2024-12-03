namespace NowProto
{
    /// <summary>
    /// Serializable NowProto message.
    /// </summary>
    public interface INowSerialize : INowMessage
    {
        /// <summary>
        /// Header flags
        /// </summary>
        ushort Flags { get; }
        /// <summary>
        ///  Serialized message body size.
        /// </summary>
        protected uint BodySize { get; }

        /// <summary>
        /// Serialize message body to the cursor.
        /// </summary>
        protected void SerializeBody(NowWriteCursor cursor);

        /// <summary>
        /// Serialized message size (with header).
        /// </summary>
        sealed uint Size => NowHeader.FixedPartSize + BodySize;

        /// <summary>
        /// Serialize complete message to the cursor.
        /// </summary>
        sealed void Serialize(NowWriteCursor cursor)
        {
            var header = new NowHeader
            {
                Size = BodySize,
                Flags = Flags,
                MsgClass = MessageClass,
                MsgKind = MessageKind,
            };

            header.Serialize(cursor);
            SerializeBody(cursor);
        }
    }
}
