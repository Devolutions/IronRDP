namespace NowProto
{
    /// <summary>
    /// Provides utilities for writing data to a buffer.
    /// </summary>
    public class NowWriteCursor(ArraySegment<byte> buffer)
    {
        public Span<byte> Remaining()
        {
            return _buffer;
        }

        public void EnsureEnoughBytes(uint length)
        {
            if ((uint)_buffer.Count < length)
            {
                throw new NowProtoException(NowProtoException.Kind.BufferTooSmall);
            }
        }

        public void Advance(uint length)
        {
            _buffer = _buffer[(int)length..];
        }

        public void WriteBytes(ReadOnlySpan<byte> data)
        {
            EnsureEnoughBytes((uint)data.Length);
            data.CopyTo(_buffer);
            Advance((uint)data.Length);
        }

        public void WriteByte(byte value)
        {
            ReadOnlySpan<byte> data = [value];

            // NOTE: Sadly, currently there is no way to tell compiler that passing
            // stackalloc memory to `ref struct` method is safe (as we are in the same
            // stack frame), so we have to duplicate code a bit instead of single (`WriteBytes`
            // call with stackalloc Span as argument)
            EnsureEnoughBytes(1);
            _buffer[0] = value;
            Advance(1);
        }

        public void WriteUint16Le(ushort value)
        {
            EnsureEnoughBytes(2);

            if (BitConverter.IsLittleEndian)
            {

                // NOTE: BitConverter.GetBytes is not used here due to fact that it returns
                // reference-counted byte[] array

                _buffer[0] = (byte)(value & 0xFF);
                _buffer[1] = (byte)((value >> 8) & 0xFF);
            }
            else
            {
                _buffer[0] = (byte)((value >> 8) & 0xFF);
                _buffer[1] = (byte)(value & 0xFF);
            }
            Advance(2);
        }

        public void WriteUint32Le(uint value)
        {
            EnsureEnoughBytes(4);
            if (BitConverter.IsLittleEndian)
            {
                _buffer[0] = (byte)(value & 0xFF);
                _buffer[1] = (byte)((value >> 8) & 0xFF);
                _buffer[2] = (byte)((value >> 16) & 0xFF);
                _buffer[3] = (byte)((value >> 24) & 0xFF);
            }
            else
            {
                _buffer[0] = (byte)((value >> 24) & 0xFF);
                _buffer[1] = (byte)((value >> 16) & 0xFF);
                _buffer[2] = (byte)((value >> 8) & 0xFF);
                _buffer[3] = (byte)(value & 0xFF);
            }
            Advance(4);
        }

        private ArraySegment<byte> _buffer = buffer;
    }
}
