using System.Runtime.InteropServices;

namespace Devolutions.IronRdp;

/// <remarks>
/// All reads share one buffer, so run one read at a time. Writes are locked and may overlap a read.
/// </remarks>
public class Framed<TS> where TS : Stream
{
    private const int ReadChunkSize = 8192;

    private readonly TS _stream;
    private readonly List<byte> _buffer;
    private readonly byte[] _readBuffer = new byte[ReadChunkSize];
    private readonly SemaphoreSlim _writeLock = new(1, 1);

    public Framed(TS stream)
    {
        _stream = stream;
        _buffer = new List<byte>();
    }

    /// <summary>
    /// Returns the underlying stream and the bytes read from it but not consumed yet.
    /// </summary>
    /// <remarks>
    /// The list is the live buffer, not a copy: later reads append to it and drop the bytes they consume.
    /// Copy it if you need a snapshot.
    /// </remarks>
    public (TS, List<byte>) GetInner()
    {
        return (_stream, _buffer);
    }

    public async Task<(Action, byte[])> ReadPdu()
    {
        var pdu = IronRdpPdu.New();
        while (true)
        {
            var pduInfo = pdu.FindSize(this._buffer.ToArray());

            // Don't remove, FindSize is generated and can return null
            if (null != pduInfo)
            {
                var frame = await this.ReadExact(pduInfo.Length);
                var action = pduInfo.Action;
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
    /// <returns>An array of bytes containing the read data.</returns>
    public async Task<byte[]> ReadExact(nuint size)
    {
        while (true)
        {
            if (_buffer.Count >= (int)size)
            {
                var res = CollectionsMarshal.AsSpan(this._buffer)[..(int)size].ToArray();
                this._buffer.RemoveRange(0, (int)size);
                return res;
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
        var size = await this._stream.ReadAsync(_readBuffer);
        this._buffer.AddRange(_readBuffer.AsSpan(0, size));
        return size;
    }

    public async Task Write(byte[] data)
    {
        await _writeLock.WaitAsync();
        try
        {
            ReadOnlyMemory<byte> memory = data;
            await _stream.WriteAsync(memory);
        }
        finally
        {
            _writeLock.Release();
        }
    }

    public async Task Write(WriteBuf buf)
    {
        var vecU8 = buf.Filled;
        var size = vecU8.Size;
        var bytesArray = new byte[size];
        vecU8.Fill(bytesArray);
        await Write(bytesArray);
    }


    /// <summary>
    /// Reads data from the buffer based on the provided PduHint.
    /// </summary>
    /// <param name="pduHint">The PduHint object used to determine the size of the data to read.</param>
    /// <returns>An asynchronous task that represents the operation. The task result contains the read data as a byte array.</returns>
    public async Task<byte[]> ReadByHint(PduHint pduHint)
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
    /// <returns>An asynchronous task that represents the operation. The task result contains the read data as a byte array.</returns>
    public async Task<byte[]> ReadByHint(IPduHint customHint)
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