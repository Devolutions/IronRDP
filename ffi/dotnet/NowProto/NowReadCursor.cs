namespace NowProto
{
    /// <summary>
    /// Provides utilities for reading data from a buffer.
    /// </summary>
    public class NowReadCursor(ArraySegment<byte> buffer)
    {
        public ReadOnlySpan<byte> Remaining()
        {
            return _buffer;
        }

        public void EnsureEnoughBytes(uint length)
        {
            if ((uint)_buffer.Count < length)
            {
                throw new NowProtoException(NowProtoException.Kind.NotEnoughData);
            }
        }

        public byte ReadByte()
        {
            var data = ReadBytes(1);
            return data[0];
        }

        public ushort ReadUInt16Le()
        {
            var data = ReadBytes(2);

            if (BitConverter.IsLittleEndian) return BitConverter.ToUInt16(data);

            Span<byte> reversed = [data[1], data[0]];
            return BitConverter.ToUInt16(reversed);

        }

        public uint ReadUInt32Le()
        {
            var data = ReadBytes(4);

            if (BitConverter.IsLittleEndian) return BitConverter.ToUInt32(data);
            Span<byte> reversed = [data[3], data[2], data[1], data[0]];
            return BitConverter.ToUInt32(reversed);

        }

        public void Advance(uint count)
        {
            var remaining = _buffer.Slice((int)count, _buffer.Count - (int)count);
            _buffer = remaining;
        }


        public ArraySegment<byte> ReadBytes(uint count)
        {
            if (_buffer.Count < count)
            {
                throw new NowProtoException(NowProtoException.Kind.NotEnoughData);
            }

            var splitPosition = (int)count;

            var data = _buffer[..splitPosition];
            var remaining = _buffer.Slice(splitPosition, _buffer.Count - splitPosition);
            _buffer = remaining;

            return data;
        }

        private ArraySegment<byte> _buffer = buffer;
    }
}
