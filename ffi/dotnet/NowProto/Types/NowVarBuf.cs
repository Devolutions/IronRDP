namespace NowProto.Types
{
    /// <summary>
    /// Buffer up to 2^31 bytes long (Length has compact variable length encoding).
    /// Implemented as class extensions for NowReadCursor and NowWriteCursor.
    ///
    /// NOW-PROTO: NOW_VARBUF
    /// </summary>
    public static class NowVarBuf
    {
        public static uint LengthOf(Span<byte> data)
        {
            return NowVarU32.LengthOf((uint)data.Length) + (uint)data.Length;
        }

        public static ArraySegment<byte> ReadVarBuf(this NowReadCursor cursor)
        {
            var length = cursor.ReadVarU32();
            return cursor.ReadBytes(length);
        }

        public static void WriteVarBuf(this NowWriteCursor cursor, ReadOnlySpan<byte> data)
        {
            cursor.WriteVarU32((uint)data.Length);
            cursor.WriteBytes(data);
        }
    }
}
