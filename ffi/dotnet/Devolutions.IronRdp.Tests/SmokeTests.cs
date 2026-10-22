namespace Devolutions.IronRdp.Tests;

internal static class SmokeTests
{
    private static readonly byte[] TpktX224ConfirmSelectingTls =
    {
        0x03, 0x00, 0x00, 0x13,
        0x0e, 0xd0, 0x00, 0x00, 0x12, 0x34, 0x00,
        0x02, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
    };

    internal static IEnumerable<(string Name, System.Action Run)> Cases()
    {
        yield return ("decoded image exposes its size and zeroed data", () =>
        {
            var image = DecodedImage.New(PixelFormat.BgrA32, 64, 32);
            Equal(64, (int)image.Width, "width");
            Equal(32, (int)image.Height, "height");

            var data = image.Data;
            Equal(64 * 32 * 4, (int)data.Size, "data size");
            var bytes = new byte[(int)data.Size + 1];
            Array.Fill(bytes, (byte)0xff);
            data.Fill(bytes);
            True(bytes[..^1].All(b => b == 0), "new image is zeroed");
            Equal((byte)0xff, bytes[^1], "bytes past the image are untouched");
        });

        yield return ("vec u8 round-trips bytes", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            Equal(3, (int)vec.Size, "size");
            var bytes = new byte[3];
            vec.Fill(bytes);
            True(bytes.SequenceEqual(new byte[] { 1, 2, 3 }), "filled bytes");
        });

        yield return ("fill leaves the rest of a larger buffer alone", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            var bytes = new byte[] { 9, 9, 9, 9, 9 };
            vec.Fill(bytes);
            True(bytes.SequenceEqual(new byte[] { 1, 2, 3, 9, 9 }), "filled bytes");
        });

        yield return ("short output buffer surfaces IronRdpException", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            var error = Throws<IronRdpException>(() => vec.Fill(new byte[2]));
            Equal(IronRdpErrorKind.Generic, error.Inner.Kind, "error kind");
            True(error.Message.Contains("buffer is too small"), $"error message: {error.Message}");
        });

        yield return ("write buf reads into a larger buffer and rejects a short one", () =>
        {
            var connector = ClientConnector.New(TlsConfig(), "127.0.0.1:33890");
            var buf = WriteBuf.New();
            var size = (int)connector.StepNoInput(buf).Size.Get();

            var bytes = new byte[size + 1];
            bytes[size] = 0xff;
            buf.ReadIntoBuf(bytes);
            Equal(0x03, (int)bytes[0], "TPKT version");
            Equal((byte)0xff, bytes[size], "bytes past the data are untouched");

            Throws<IronRdpException>(() => buf.ReadIntoBuf(new byte[size - 1]));
        });

        yield return ("pdu find_size reads the TPKT length", () =>
        {
            var pdu = IronRdpPdu.New();
            True(pdu.FindSize(TpktX224ConfirmSelectingTls[..2]) is null, "partial header has no size");

            PduInfo? info = pdu.FindSize(TpktX224ConfirmSelectingTls);
            True(info is not null, "full PDU has a size");
            Equal(TpktX224ConfirmSelectingTls.Length, (int)info!.Length, "PDU length");
        });

        yield return ("framed serializes writes and releases its lock on another thread", () =>
        {
            var stream = new PendingWriteStream();
            var framed = new Framed<PendingWriteStream>(stream);

            var first = framed.Write(new byte[] { 1, 2 });
            var second = framed.Write(new byte[] { 3 });
            True(!first.IsCompleted, "first write waits for the stream");
            Equal(2, (int)stream.Length, "second write waits for the first");

            stream.FinishOnAnotherThread();
            first.GetAwaiter().GetResult();
            second.GetAwaiter().GetResult();
            True(stream.ToArray().SequenceEqual(new byte[] { 1, 2, 3 }), "written bytes");
        });

        yield return ("config builder reports the first missing field", () =>
        {
            var builder = ConfigBuilder.New();
            var error = Throws<IronRdpException>(() => builder.Build());
            True(error.Message.Contains("credentials not set"), $"error message: {error.Message}");
        });

        yield return ("connector sends an X.224 request and accepts the confirm", () =>
        {
            var config = TlsConfig();
            var connector = ClientConnector.New(config, "127.0.0.1:33890");
            var buf = WriteBuf.New();

            var size = connector.StepNoInput(buf).Size;
            True(size.IsSome() && size.Get() > 0, "connection request was written");

            var filled = buf.Filled;
            var request = new byte[(int)filled.Size];
            filled.Fill(request);
            Equal(0x03, (int)request[0], "TPKT version");

            var hint = connector.NextPduHint();
            True(hint is not null, "connector waits for the confirm");
            True(!hint!.FindSize(TpktX224ConfirmSelectingTls[..2]).IsSome(), "partial confirm has no size");
            Equal(TpktX224ConfirmSelectingTls.Length, (int)hint.FindSize(TpktX224ConfirmSelectingTls).Get(), "confirm size");

            buf.Clear();
            connector.Step(TpktX224ConfirmSelectingTls, buf);

            True(connector.ShouldPerformSecurityUpgrade(), "TLS upgrade is next");
            var state = connector.DynState;
            True(!string.IsNullOrEmpty(state.Name), "state has a name");
            True(!state.IsTerminal(), "state is not terminal");
        });

        if (OperatingSystem.IsWindows())
        {
            yield return ("windows clipboard with file transfer starts idle", () =>
            {
                var clipboard = WinCliprdr.NewWithFileTransfer(true);
                True(clipboard.BackendFactory() != null, "backend factory");
                True(clipboard.NextClipboardMessageTimeout(10) == null, "no message before a session");
            });
        }
    }

    private static Config TlsConfig()
    {
        var builder = ConfigBuilder.New();
        builder.WithUsernameAndPassword("user", "password");
        builder.SetDesktopSize(600, 800);
        builder.ClientName = "smoke-tests";
        builder.ClientDir = "C:\\Windows\\System32\\mstscax.dll";
        builder.EnableTls = true;
        builder.EnableCredssp = false;
        builder.PerformanceFlags = PerformanceFlags.NewDefault();
        return builder.Build();
    }

    private sealed class PendingWriteStream : MemoryStream
    {
        private readonly TaskCompletionSource _written = new();

        public override ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default)
        {
            Write(buffer.Span);
            return new ValueTask(_written.Task);
        }

        public void FinishOnAnotherThread()
        {
            var thread = new Thread(() => _written.SetResult());
            thread.Start();
            thread.Join();
        }
    }

    private static T Throws<T>(System.Action action) where T : Exception
    {
        try
        {
            action();
        }
        catch (T error)
        {
            return error;
        }

        throw new InvalidOperationException($"expected {typeof(T).Name}");
    }

    private static void True(bool condition, string what)
    {
        if (!condition)
            throw new InvalidOperationException($"{what}: expected true");
    }

    private static void Equal<T>(T expected, T actual, string what)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
            throw new InvalidOperationException($"{what}: expected {expected}, got {actual}");
    }
}
