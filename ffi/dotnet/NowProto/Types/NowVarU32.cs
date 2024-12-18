namespace NowProto.Types
{
    /// <summary>
    /// Variable-length encoded u32.
    /// Value range: `[0..0x3FFFFFFF]`
    /// Implemented as class extensions for NowReadCursor and NowWriteCursor.
    ///
    /// NOW-PROTO: NOW_VARU32
    /// </summary>
    public static class NowVarU32
    {
        public static uint LengthOf(uint value)
        {
            return value switch
            {
                <= 0x3F => 1,
                <= 0x3FFF => 2,
                <= 0x3FFFFF => 3,
                <= 0x3FFFFFFF => 4,
                _ => throw new NowProtoException(NowProtoException.Kind.VarU32OutOfRange)
            };
        }

        public static uint ReadVarU32(this NowReadCursor cursor)
        {
            var header = cursor.ReadByte();
            var c = (byte)(header >> 6 & 0x03);

            if (c == 0)
            {
                return (uint)(header & 0x3F);
            }

            var bytes = cursor.ReadBytes(c);

            // Read most significant byte from header byte
            var val1 = (byte)(header & 0x3F);
            var shift = c * 8;
            var num = (uint)val1 << shift;

            // Read val2..valN
            foreach (var current in bytes)
            {
                shift -= 8;
                num |= (uint)current << shift;
            }

            return num;
        }

        public static void WriteVarU32(this NowWriteCursor cursor, uint num)
        {
            var encodedSize = LengthOf(num);
            var shift = (int)(encodedSize - 1) * 8;

            cursor.EnsureEnoughBytes(encodedSize);

            for (var i = 0; i < encodedSize; i++)
            {
                var b = (byte)(num >> shift & 0xFF);
                cursor.Remaining()[i] = b;

                if (shift != 0)
                {
                    shift -= 8;
                }
            }

            var c = (byte)(encodedSize - 1);
            cursor.Remaining()[0] |= (byte)(c << 6);

            cursor.Advance(encodedSize);
        }
    }
}
