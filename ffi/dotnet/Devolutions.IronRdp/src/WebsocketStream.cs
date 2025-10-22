using System;
using System.Buffers;
using System.IO;
using System.Net.WebSockets;
using System.Threading;
using System.Threading.Tasks;

public sealed class WebSocketStream : Stream
{
    private readonly ClientWebSocket _ws;
    private readonly byte[] _recvBuf;
    private int _recvPos;
    private int _recvLen;
    private bool _remoteClosed;
    private bool _disposed;

    private const int DefaultRecvBufferSize = 64 * 1024;
    private const int MaxSendFrame = 16 * 1024; // send in chunks

    private WebSocketStream(ClientWebSocket ws, int receiveBufferSize)
    {
        _ws = ws ?? throw new ArgumentNullException(nameof(ws));
        _recvBuf = ArrayPool<byte>.Shared.Rent(Math.Max(1024, receiveBufferSize));
    }

    public static async Task<WebSocketStream> ConnectAsync(
        Uri uri,
        ClientWebSocket? ws = null,
        int receiveBufferSize = DefaultRecvBufferSize,
        CancellationToken ct = default)
    {
        ws ??= new ClientWebSocket();
        await ws.ConnectAsync(uri, ct).ConfigureAwait(false);
        return new WebSocketStream(ws, receiveBufferSize);
    }

    public ClientWebSocket Socket => _ws;

    public override bool CanRead => true;
    public override bool CanSeek => false;
    public override bool CanWrite => true;
    public override long Length => throw new NotSupportedException();
    public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }

    public override void Flush() { /* no-op */ }
    public override Task FlushAsync(CancellationToken cancellationToken) => Task.CompletedTask;

    public override int Read(byte[] buffer, int offset, int count) =>
        ReadAsync(buffer.AsMemory(offset, count)).AsTask().GetAwaiter().GetResult();

    public override void Write(byte[] buffer, int offset, int count) =>
        WriteAsync(buffer.AsMemory(offset, count)).GetAwaiter().GetResult();

    public override async ValueTask<int> ReadAsync(
        Memory<byte> destination, CancellationToken cancellationToken = default)
    {
        if (_disposed) throw new ObjectDisposedException(nameof(WebSocketStream));
        if (_remoteClosed) return 0;
        if (destination.Length == 0) return 0;

        // Fill local buffer if empty
        if (_recvLen == 0)
        {
            var mem = _recvBuf.AsMemory();
            while (true)
            {
                var result = await _ws.ReceiveAsync(mem, cancellationToken).ConfigureAwait(false);

                // Close frame → signal EOF
                if (result.MessageType == WebSocketMessageType.Close)
                {
                    _remoteClosed = true;
                    try { await _ws.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "OK", cancellationToken).ConfigureAwait(false); }
                    catch { /* ignore */ }
                    return 0;
                }

                if (result.MessageType == WebSocketMessageType.Text)
                    throw new InvalidOperationException("Received TEXT frame; this stream expects BINARY.");

                // Some data arrived
                if (result.Count > 0)
                {
                    _recvPos = 0;
                    _recvLen = result.Count;
                    break;
                }

                // Keep looping if Count == 0 (can happen with pings/keepers)
            }
        }

        var toCopy = Math.Min(destination.Length, _recvLen);
        new ReadOnlySpan<byte>(_recvBuf, _recvPos, toCopy).CopyTo(destination.Span);
        _recvPos += toCopy;
        _recvLen -= toCopy;

        // If we've drained local buffer, try to prefetch next chunk (non-blocking behavior not guaranteed)
        if (_recvLen == 0 && _ws.State == WebSocketState.Open)
        {
            // optional prefetch: not strictly necessary—kept simple
        }

        return toCopy;
    }

    public override async Task WriteAsync(
        byte[] buffer, int offset, int count, CancellationToken cancellationToken)
        => await WriteAsync(buffer.AsMemory(offset, count), cancellationToken);

    public override async ValueTask WriteAsync(
        ReadOnlyMemory<byte> source, CancellationToken cancellationToken = default)
    {
        if (_disposed) throw new ObjectDisposedException(nameof(WebSocketStream));
        if (_ws.State != WebSocketState.Open) throw new IOException("WebSocket is not open.");

        // Treat each Write* as one complete WebSocket message (Binary).
        // Chunk large payloads as continuation frames and set EndOfMessage on the last chunk.
        int sent = 0;
        while (sent < source.Length)
        {
            var chunkLen = Math.Min(MaxSendFrame, source.Length - sent);
            var chunk = source.Slice(sent, chunkLen);
            sent += chunkLen;

            bool end = (sent == source.Length);
            await _ws.SendAsync(chunk, WebSocketMessageType.Binary, end, cancellationToken).ConfigureAwait(false);
        }
    }

    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
    public override void SetLength(long value) => throw new NotSupportedException();

    protected override void Dispose(bool disposing)
    {
        if (_disposed) return;
        if (disposing)
        {
            try
            {
                if (_ws.State == WebSocketState.Open)
                {
                    _ws.CloseAsync(WebSocketCloseStatus.NormalClosure, "Disposing", CancellationToken.None)
                       .GetAwaiter().GetResult();
                }
            }
            catch { /* ignore on dispose */ }
            _ws.Dispose();
            ArrayPool<byte>.Shared.Return(_recvBuf);
        }
        _disposed = true;
        base.Dispose(disposing);
    }

#if NETSTANDARD2_1_OR_GREATER || NET5_0_OR_GREATER
    public override async ValueTask DisposeAsync()
    {
        if (!_disposed)
        {
            try
            {
                if (_ws.State == WebSocketState.Open)
                    await _ws.CloseAsync(WebSocketCloseStatus.NormalClosure, "Disposing", CancellationToken.None).ConfigureAwait(false);
            }
            catch { /* ignore */ }
            _ws.Dispose();
            ArrayPool<byte>.Shared.Return(_recvBuf);
            _disposed = true;
        }
        await base.DisposeAsync().ConfigureAwait(false);
    }
#endif
}
