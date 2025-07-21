using System;
using System.Diagnostics;
using System.IO;
using System.Net.WebSockets;
using System.Threading;
using System.Threading.Tasks;

namespace Devolutions.IronRdp.src
{
    public class WebsocketStream : Stream
    {
        private readonly ClientWebSocket _webSocket;

        public WebsocketStream(ClientWebSocket webSocket)
        {
            _webSocket = webSocket;
        }

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => true;
        public override long Length => throw new NotSupportedException();
        public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }

        public override async Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken)
        {
            var result = await _webSocket.ReceiveAsync(
                new ArraySegment<byte>(buffer, offset, count), cancellationToken);

            if (result.CloseStatus.HasValue)   // remote sent a CLOSE frame
            {
                return 0;                      // treat as end-of-stream
            }

            return result.Count;
        }

        public override async Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken)
        {
            if (count == 0)
            {
                // Note: this is particularly important, if we send a zero-length frame,
                // somehow Gateway will raise TLS issue during the proxy.
                return; // Nothing to write
            }

            await _webSocket.SendAsync(
                new ArraySegment<byte>(buffer, offset, count),
                WebSocketMessageType.Binary,
                true,
                cancellationToken);
        }

        public override void Flush()
        {
            // No need. the third parameter of SendAsync is set to true, which means the frame is sent immediately.
            // Also, this method is not called in practice ever somehow. 
            // However, since it's not blocking any functionality, we can leave it empty.
        }

        // Not supported
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }
}
