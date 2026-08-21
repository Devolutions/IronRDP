using System.Runtime.InteropServices;

namespace Devolutions.IronRdp;

/// <summary>
/// Bytes a <c>Framed</c> had buffered when it was taken apart, along with when they arrived.
/// </summary>
/// <remarks>
/// Handing this back to <c>Framed(TS, Leftover)</c> is the only way to rebuild a framed stream
/// without losing either the bytes or their arrival time. Keeping the two together is what makes
/// "the buffer is non-empty, so we know when it was read" true by construction.
/// </remarks>
public sealed class Leftover
{
    internal List<byte> Bytes { get; }

    internal MonotonicInstant? ReadAt { get; }

    internal Leftover(List<byte> bytes, MonotonicInstant? readAt)
    {
        Bytes = bytes;
        ReadAt = readAt;
    }

    /// <summary>
    /// Nothing was buffered.
    /// </summary>
    public static Leftover None()
    {
        return new Leftover(new List<byte>(), null);
    }

    public bool IsEmpty => Bytes.Count == 0;
}

public class Framed<TS> where TS : Stream
{
    private readonly TS _stream;
    private List<byte> _buffer;
    private readonly Mutex _writeLock = new();

    /// <summary>
    /// When the most recent read completed.
    /// </summary>
    /// <remarks>
    /// INVARIANT: this is set whenever <c>_buffer</c> is non-empty, which is what lets the read
    /// methods return an arrival time without a null check the caller could get wrong.
    /// <br/>
    /// The same instance is handed to every PDU that came out of that read, so callers must not
    /// dispose it.
    /// </remarks>
    private MonotonicInstant? _lastReadAt;

    public Framed(TS stream) : this(stream, Leftover.None())
    {
    }

    public Framed(TS stream, Leftover leftover)
    {
        _stream = stream;
        _buffer = leftover.Bytes;
        _lastReadAt = leftover.ReadAt;
    }

    /// <summary>
    /// Takes the framed stream apart, so that a new one can be built over the same bytes.
    /// </summary>
    public (TS, Leftover) GetInner()
    {
        return (_stream, new Leftover(new List<byte>(_buffer), _lastReadAt));
    }

    public async Task<(Action, byte[])> ReadPdu()
    {
        while (true)
        {
            var pduInfo = IronRdpPdu.New().FindSize(this._buffer.ToArray());

            // Don't remove, FindSize is generated and can return null
            if (null != pduInfo)
            {
                var (frame, _) = await this.ReadExact(pduInfo.GetLength());
                var action = pduInfo.GetAction();
                return (action, frame);
            }
            else
            {
                var len = await this.Read();
                if (len == 0)
                {
                    throw new IronRdpLibException(IronRdpLibExceptionType.EndOfFile, "EOF on ReadPdu");
                }
            }
        }
    }

    /// <summary>
    /// Returns a span that represents a portion of the underlying buffer without modifying it.
    /// </summary>
    /// <remarks>Memory safety: the Framed instance should not be modified (any read operations) while span is in use.</remarks>
    /// <returns>A span that represents a portion of the underlying buffer.</returns>
    public Span<byte> Peek()
    {
        return CollectionsMarshal.AsSpan(this._buffer);
    }

    /// <summary>
    /// Reads from 0 to size bytes from the front of the buffer, and remove them from the buffer keeping the rest.
    /// </summary>
    /// <param name="size">The number of bytes to read.</param>
    /// <returns>
    /// The bytes read, and when the read that completed them finished. The instant is shared with
    /// every other PDU from that same read, so do not dispose it.
    /// </returns>
    public async Task<(byte[], MonotonicInstant)> ReadExact(nuint size)
    {
        while (true)
        {
            if (_lastReadAt is { } readAt && _buffer.Count >= (int)size)
            {
                var res = this._buffer.Take((int)size).ToArray();
                this._buffer = this._buffer.Skip((int)size).ToList();
                return (res, readAt);
            }

            var len = await this.Read();
            if (len == 0)
            {
                throw new Exception("EOF");
            }
        }
    }

    async Task<int> Read()
    {
        var buffer = new byte[8096];
        Memory<byte> memory = buffer;
        var size = await this._stream.ReadAsync(memory);

        if (size > 0)
        {
            // Stamp here rather than when the PDU is eventually handed to a sequence: the two can
            // be far apart, and only this is when the bytes actually arrived.
            this._lastReadAt = MonotonicInstant.Now();
            this._buffer.AddRange(buffer.Take(size));
        }

        return size;
    }

    public async Task Write(byte[] data)
    {
        _writeLock.WaitOne();
        try
        {
            ReadOnlyMemory<byte> memory = data;
            await _stream.WriteAsync(memory);
        }
        finally
        {
            _writeLock.ReleaseMutex();
        }
    }

    public async Task Write(WriteBuf buf)
    {
        var vecU8 = buf.GetFilled();
        var size = vecU8.GetSize();
        var bytesArray = new byte[size];
        vecU8.Fill(bytesArray);
        await Write(bytesArray);
    }


    /// <summary>
    /// Reads data from the buffer based on the provided PduHint.
    /// </summary>
    /// <param name="pduHint">The PduHint object used to determine the size of the data to read.</param>
    /// <returns>
    /// The PDU bytes, and when the read that completed them finished. The instant is shared with
    /// every other PDU from that same read, so do not dispose it.
    /// </returns>
    public async Task<(byte[], MonotonicInstant)> ReadByHint(PduHint pduHint)
    {
        while (true)
        {
            var size = pduHint.FindSize(this._buffer.ToArray());
            if (size.IsSome())
            {
                return await this.ReadExact(size.Get());
            }
            else
            {
                var len = await this.Read();
                if (len == 0)
                {
                    throw new Exception("EOF");
                }
            }
        }
    }

    /// <summary>
    /// Reads data from the buffer based on a custom PDU hint function.
    /// </summary>
    /// <param name="customHint">A custom hint object implementing IPduHint interface.</param>
    /// <returns>
    /// The PDU bytes, and when the read that completed them finished. The instant is shared with
    /// every other PDU from that same read, so do not dispose it.
    /// </returns>
    public async Task<(byte[], MonotonicInstant)> ReadByHint(IPduHint customHint)
    {
        while (true)
        {
            var result = customHint.FindSize(this._buffer.ToArray());
            if (result.HasValue)
            {
                return await this.ReadExact((nuint)result.Value.Item2);
            }
            else
            {
                var len = await this.Read();
                if (len == 0)
                {
                    throw new Exception("EOF");
                }
            }
        }
    }
}

/// <summary>
/// Interface for custom PDU hint implementations.
/// </summary>
public interface IPduHint
{
    /// <summary>
    /// Finds the size of a PDU in the given byte array.
    /// </summary>
    /// <param name="bytes">The byte array to analyze.</param>
    /// <returns>
    /// A tuple (detected, size) if PDU is detected, null if more bytes are needed.
    /// Throws exception if invalid PDU is detected.
    /// </returns>
    (bool, int)? FindSize(byte[] bytes);
}