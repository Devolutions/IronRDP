using System.Runtime.InteropServices;

namespace Devolutions.IronRdp;

/// <summary>
/// Splits a stream into RDP PDUs.
/// </summary>
/// <remarks>
/// <para>
/// All reads share one buffer, so run one read at a time; <see cref="TryReadBufferedPdu"/> and <see cref="Peek"/>
/// are reads too. Writes are locked and may overlap a read.
/// </para>
/// <para>
/// <see cref="ReadPdu()"/> sizes an active-stage PDU from the first <see cref="HeaderProbeBytes"/> buffered bytes,
/// so its cost does not grow with the bytes buffered behind it. <see cref="ReadByHint(PduHint)"/> hands the hint
/// everything buffered, because a connection-sequence hint may need more than a fixed header and those PDUs are
/// few and small.
/// </para>
/// <para>
/// Every read reports a stream that ends before the PDU is complete, in its header or its body, as
/// <see cref="IronRdpLibException"/> with <see cref="IronRdpLibExceptionType.EndOfFile"/>.
/// </para>
/// </remarks>
public class Framed<TS> where TS : Stream
{
    private const int ReadChunkSize = 8192;

    // An active-stage PDU announces its length in its first bytes: four for TPKT (MS-RDPBCGR 2.2.9.1.1),
    // at most three for fast-path (MS-RDPBCGR 2.2.9.1.2).
    private const int HeaderProbeBytes = 4;

    private readonly TS _stream;
    private readonly List<byte> _buffer;
    private readonly byte[] _readBuffer = new byte[ReadChunkSize];
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly IronRdpPdu _pdu = IronRdpPdu.New();

    // The stream read in flight. A cancelled read leaves it here, and the next read finishes it first, so the bytes
    // it delivers are neither lost nor read twice.
    private Task<int>? _pendingRead;

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

    /// <summary>
    /// Reads the next active-stage PDU, reading from the stream until it is complete.
    /// </summary>
    public Task<(Action Action, byte[] Payload)> ReadPdu()
    {
        return ReadPdu(CancellationToken.None);
    }

    /// <summary>
    /// Reads the next active-stage PDU, reading from the stream until it is complete.
    /// </summary>
    /// <remarks>
    /// Cancellation stops waiting but leaves the stream read in flight; the next read, asynchronous or synchronous,
    /// finishes it first, so bytes already buffered or still arriving stay available in order.
    /// </remarks>
    public async Task<(Action Action, byte[] Payload)> ReadPdu(CancellationToken cancellationToken)
    {
        (Action Action, byte[] Payload) pdu;
        while (!TryReadBufferedPdu(out pdu))
        {
            if (await Read(cancellationToken) == 0)
            {
                throw EndOfFile();
            }
        }

        return pdu;
    }

    /// <summary>
    /// Reads the next active-stage PDU, blocking the calling thread until it is complete.
    /// </summary>
    /// <remarks>
    /// For a read loop on a dedicated thread; closing the stream unblocks it.
    /// </remarks>
    public (Action Action, byte[] Payload) ReadPduSync()
    {
        (Action Action, byte[] Payload) pdu;
        while (!TryReadBufferedPdu(out pdu))
        {
            if (ReadSync() == 0)
            {
                throw EndOfFile();
            }
        }

        return pdu;
    }

    /// <summary>
    /// Takes the next active-stage PDU when it is already buffered, without reading from the stream.
    /// </summary>
    /// <remarks>
    /// This is a read: it consumes from the shared buffer, so do not call it while a <c>ReadPdu</c>, <c>ReadExact</c>
    /// or <c>ReadByHint</c> is pending. Call it between reads of the same loop.
    /// </remarks>
    /// <returns>False when the buffer holds no complete PDU; the buffer is left as it was.</returns>
    public bool TryReadBufferedPdu(out (Action Action, byte[] Payload) pdu)
    {
        pdu = default;
        var buffered = CollectionsMarshal.AsSpan(_buffer);
        if (buffered.IsEmpty)
        {
            return false;
        }

        var probeLength = Math.Min(buffered.Length, HeaderProbeBytes);
        var info = _pdu.FindSize(buffered[..probeLength].ToArray());
        if (info is null)
        {
            return false;
        }

        var length = checked((int)info.Length);
        if (buffered.Length < length)
        {
            return false;
        }

        pdu = (info.Action, buffered[..length].ToArray());
        _buffer.RemoveRange(0, length);
        return true;
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
    public Task<byte[]> ReadExact(nuint size)
    {
        return ReadExact(size, CancellationToken.None);
    }

    /// <inheritdoc cref="ReadExact(nuint)"/>
    public async Task<byte[]> ReadExact(nuint size, CancellationToken cancellationToken)
    {
        while (true)
        {
            if (_buffer.Count >= (int)size)
            {
                var res = CollectionsMarshal.AsSpan(this._buffer)[..(int)size].ToArray();
                this._buffer.RemoveRange(0, (int)size);
                return res;
            }

            var len = await this.Read(cancellationToken);
            if (len == 0)
            {
                throw EndOfFile();
            }
        }
    }

    private static IronRdpLibException EndOfFile()
    {
        return new IronRdpLibException(IronRdpLibExceptionType.EndOfFile, "stream ended before the PDU was complete");
    }

    async Task<int> Read(CancellationToken cancellationToken)
    {
        var pending = _pendingRead ??= _stream.ReadAsync(_readBuffer.AsMemory(), CancellationToken.None).AsTask();
        int size;
        try
        {
            size = await pending.WaitAsync(cancellationToken);
        }
        catch when (pending.IsFaulted || pending.IsCanceled)
        {
            _pendingRead = null;
            throw;
        }

        _pendingRead = null;
        this._buffer.AddRange(_readBuffer.AsSpan(0, size));
        return size;
    }

    int ReadSync()
    {
        int size;
        if (_pendingRead is { } pending)
        {
            _pendingRead = null;
            size = pending.GetAwaiter().GetResult();
        }
        else
        {
            size = this._stream.Read(_readBuffer, 0, _readBuffer.Length);
        }

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
    public Task<byte[]> ReadByHint(PduHint pduHint)
    {
        return ReadByHint(pduHint, CancellationToken.None);
    }

    /// <inheritdoc cref="ReadByHint(PduHint)"/>
    public async Task<byte[]> ReadByHint(PduHint pduHint, CancellationToken cancellationToken)
    {
        while (true)
        {
            // Connection-time PDUs are few and small, so the hint sees everything buffered.
            var size = pduHint.FindSize(this._buffer.ToArray());
            if (size.IsSome())
            {
                return await this.ReadExact(size.Get(), cancellationToken);
            }
            else
            {
                var len = await this.Read(cancellationToken);
                if (len == 0)
                {
                    throw EndOfFile();
                }
            }
        }
    }

    /// <summary>
    /// Reads data from the buffer based on a custom PDU hint function.
    /// </summary>
    /// <param name="customHint">A custom hint object implementing IPduHint interface.</param>
    /// <returns>An asynchronous task that represents the operation. The task result contains the read data as a byte array.</returns>
    public Task<byte[]> ReadByHint(IPduHint customHint)
    {
        return ReadByHint(customHint, CancellationToken.None);
    }

    /// <inheritdoc cref="ReadByHint(IPduHint)"/>
    public async Task<byte[]> ReadByHint(IPduHint customHint, CancellationToken cancellationToken)
    {
        while (true)
        {
            var result = customHint.FindSize(this._buffer.ToArray());
            if (result.HasValue)
            {
                return await this.ReadExact((nuint)result.Value.Item2, cancellationToken);
            }
            else
            {
                var len = await this.Read(cancellationToken);
                if (len == 0)
                {
                    throw EndOfFile();
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
