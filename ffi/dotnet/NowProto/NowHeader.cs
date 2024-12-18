namespace NowProto
{
    /// <summary>
    /// The NOW_HEADER structure is the header common to all NOW protocol messages.
    ///
    /// NOW-PROTO: NOW_HEADER
    /// </summary>
    internal record struct NowHeader(uint Size)
    {
        public static NowHeader Deserialize(NowReadCursor reader)
        {
            reader.EnsureEnoughBytes(FixedPartSize);
            var size = reader.ReadUInt32Le();
            var msgClass = reader.ReadByte();
            var msgKind = reader.ReadByte();
            var flags = reader.ReadUInt16Le();

            return new NowHeader
            {
                Size = size,
                MsgClass = msgClass,
                MsgKind = msgKind,
                Flags = flags
            };
        }

        public void Serialize(NowWriteCursor writer)
        {
            writer.EnsureEnoughBytes(FixedPartSize);
            writer.WriteUint32Le(Size);
            writer.WriteByte(MsgClass);
            writer.WriteByte(MsgKind);
            writer.WriteUint16Le(Flags);
        }

        internal const uint FixedPartSize = 8;

        public uint Size = Size;
        public ushort Flags = 0;
        public byte MsgClass = 0;
        public byte MsgKind = 0;
    }
}
