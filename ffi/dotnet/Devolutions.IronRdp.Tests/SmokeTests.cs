namespace Devolutions.IronRdp.Tests;

internal static class SmokeTests
{
    private static readonly byte[] TpktX224ConfirmSelectingTls =
    {
        0x03, 0x00, 0x00, 0x13,
        0x0e, 0xd0, 0x00, 0x00, 0x12, 0x34, 0x00,
        0x02, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
    };

    private static readonly byte[] TpktX224ConfirmWithoutNegotiation =
    {
        0x03, 0x00, 0x00, 0x0b,
        0x06, 0xd0, 0x00, 0x00, 0x56, 0x78, 0x00,
    };

    // Fast-path update PDUs with a one-byte and a two-byte length1/length2 encoding (MS-RDPBCGR §2.2.9.1.2).
    private static readonly byte[] FastPathShortLength = { 0x00, 0x05, 0x01, 0x02, 0x03 };
    private static readonly byte[] FastPathLongLength = { 0x00, 0x80, 0x06, 0x01, 0x02, 0x03 };

    internal static IEnumerable<(string Name, System.Action Run)> Cases()
    {
        // oracle: ffi/INTENT.md "Copying into caller buffers" (bytes past the image)
        yield return ("decoded image exposes its size and zeroed data", () =>
        {
            var image = DecodedImage.New(PixelFormat.BgrA32, 64, 32);
            Equal(64, (int)image.Width, "width");
            Equal(32, (int)image.Height, "height");

            Equal(64 * 32 * 4, (int)image.DataSize, "data size");
            var bytes = new byte[(int)image.DataSize + 1];
            Array.Fill(bytes, (byte)0xff);
            image.FillData(bytes);
            True(bytes[..^1].All(b => b == 0), "new image is zeroed");
            Equal((byte)0xff, bytes[^1], "bytes past the image are untouched");
        });

        // oracle: ffi/INTENT.md "Copying into caller buffers"
        yield return ("decoded image rejects a short output buffer", () =>
        {
            var image = DecodedImage.New(PixelFormat.BgrA32, 64, 32);
            var error = Throws<IronRdpException>(() => image.FillData(new byte[4]));
            Equal(IronRdpErrorKind.Generic, error.Inner.Kind, "error kind");
            True(error.Message.Contains("buffer is too small"), $"error message: {error.Message}");
        });

        // oracle: ffi/README.md lists DecodedImage as IDisposable; Diplomat's generated Dispose remarks say later calls throw ObjectDisposedException
        yield return ("disposed decoded image throws ObjectDisposedException", () =>
        {
            var image = DecodedImage.New(PixelFormat.BgrA32, 64, 32);
            image.Dispose();
            Throws<ObjectDisposedException>(() => _ = image.DataSize);
        });

        // oracle: mutant source and destination row offsets and inclusive bounds; the rectangle is the GraphicsUpdate IronRDP reports
        yield return ("decoded image copies only the damaged rectangle", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var damage = PaintSurface(stage, image, 2, 1, 3, 2);
            Equal((2, 1, 4, 2), ((int)damage.Left, (int)damage.Top, (int)damage.Right, (int)damage.Bottom), "damage");
            const int stride = 8 * 4 + 4;
            var buffer = new byte[stride * 4];
            Array.Fill(buffer, (byte)0xff);

            image.CopyRectInto(damage, buffer, stride);

            for (int y = 0; y < 4; y++)
            {
                for (int x = 0; x < stride; x++)
                {
                    bool inside = y is >= 1 and <= 2 && x >= 2 * 4 && x < 5 * 4;
                    byte expected = inside ? SurfaceByte(3, x - 2 * 4, y - 1) : (byte)0xff;
                    Equal(expected, buffer[y * stride + x], $"byte ({x}, {y})");
                }
            }
        });

        // oracle: intent ffi/INTENT.md §copying-a-rectangle
        yield return ("decoded image copy rejects a rectangle outside the image", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var full = PaintSurface(stage, image, 0, 0, 8, 4);
            var buffer = new byte[8 * 4 * 4];
            Array.Fill(buffer, (byte)0xff);
            var narrow = DecodedImage.New(PixelFormat.BgrX32, 7, 4);
            Throws<IronRdpException>(() => narrow.CopyRectInto(full, buffer, 8 * 4));
            var shallow = DecodedImage.New(PixelFormat.BgrX32, 8, 3);
            Throws<IronRdpException>(() => shallow.CopyRectInto(full, buffer, 8 * 4));
            True(buffer.All(b => b == 0xff), "buffer untouched");
        });

        // oracle: intent ffi/INTENT.md §copying-a-rectangle
        yield return ("decoded image copy rejects a short stride", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var full = PaintSurface(stage, image, 0, 0, 8, 4);
            var buffer = new byte[8 * 4 * 4];
            Array.Fill(buffer, (byte)0xff);
            Throws<IronRdpException>(() => image.CopyRectInto(full, buffer, 8 * 4 - 1));
            True(buffer.All(b => b == 0xff), "buffer untouched");
        });

        // oracle: intent ffi/INTENT.md §copying-a-rectangle
        yield return ("decoded image copy rejects a short buffer", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var full = PaintSurface(stage, image, 0, 0, 8, 4);
            const int stride = 8 * 4;
            var buffer = new byte[stride * 3];
            Array.Fill(buffer, (byte)0xff);
            Throws<IronRdpException>(() => image.CopyRectInto(full, buffer, stride));
            True(buffer.All(b => b == 0xff), "buffer untouched");
        });

        // oracle: intent ffi/INTENT.md §copying-a-rectangle
        yield return ("decoded image copy rejects a stride whose last row overflows", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var full = PaintSurface(stage, image, 0, 0, 8, 4);
            var buffer = new byte[8 * 4 * 4];
            Array.Fill(buffer, (byte)0xff);
            Throws<IronRdpException>(() => image.CopyRectInto(full, buffer, nuint.MaxValue));
            True(buffer.All(b => b == 0xff), "buffer untouched");
        });

        yield return ("vec u8 round-trips bytes", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            Equal(3, (int)vec.Size, "size");
            var bytes = new byte[3];
            vec.Fill(bytes);
            True(bytes.SequenceEqual(new byte[] { 1, 2, 3 }), "filled bytes");
        });

        // oracle: ffi/INTENT.md "Copying into caller buffers"
        yield return ("fill leaves the rest of a larger buffer alone", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            var bytes = new byte[] { 9, 9, 9, 9, 9 };
            vec.Fill(bytes);
            True(bytes.SequenceEqual(new byte[] { 1, 2, 3, 9, 9 }), "filled bytes");
        });

        // oracle: ffi/INTENT.md "Copying into caller buffers"
        yield return ("short output buffer surfaces IronRdpException", () =>
        {
            var vec = VecU8.FromBytes(new byte[] { 1, 2, 3 });
            var error = Throws<IronRdpException>(() => vec.Fill(new byte[2]));
            Equal(IronRdpErrorKind.Generic, error.Inner.Kind, "error kind");
            True(error.Message.Contains("buffer is too small"), $"error message: {error.Message}");
        });

        // oracle: ffi/INTENT.md "Copying into caller buffers"
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

        // oracle: spec MS-RDPBCGR §2.2.1.2 (each X.224 Connection Confirm is one TPKT-framed PDU)
        yield return ("framed reads back-to-back PDUs", () =>
        {
            var bytes = TpktX224ConfirmSelectingTls.Concat(TpktX224ConfirmWithoutNegotiation).ToArray();
            var stream = new ChunkedReadStream(bytes, 2, 21, 7);
            var framed = new Framed<ChunkedReadStream>(stream);

            var (_, first) = framed.ReadPdu().GetAwaiter().GetResult();
            True(first.SequenceEqual(TpktX224ConfirmSelectingTls), "first PDU bytes");

            var (_, second) = framed.ReadPdu().GetAwaiter().GetResult();
            True(second.SequenceEqual(TpktX224ConfirmWithoutNegotiation), "second PDU bytes");

            True(framed.Peek().IsEmpty, "nothing left in the buffer");
        });

        // oracle: spec MS-RDPBCGR §2.2.9.1.1 (TPKT header is four bytes) and §2.2.9.1.2 (fast-path header is at most three)
        yield return ("pdu find_size sizes a PDU from its header alone", () =>
        {
            var pdu = IronRdpPdu.New();
            Equal(TpktX224ConfirmSelectingTls.Length, (int)pdu.FindSize(TpktX224ConfirmSelectingTls[..4])!.Length, "TPKT length");
            Equal(FastPathShortLength.Length, (int)pdu.FindSize(FastPathShortLength[..2])!.Length, "one-byte fast-path length");
            Equal(FastPathLongLength.Length, (int)pdu.FindSize(FastPathLongLength[..3])!.Length, "two-byte fast-path length");
            True(pdu.FindSize(FastPathLongLength[..2]) is null, "two-byte fast-path length needs its second byte");
        });

        // oracle: spec MS-RDPBCGR §2.2.9.1.1 and §2.2.9.1.2 for the PDU boundaries; mutant for the ReadCount
        // assertions, which pin that a buffered PDU never reads the stream
        yield return ("framed takes buffered PDUs without reading again", () =>
        {
            var bytes = TpktX224ConfirmSelectingTls.Concat(FastPathLongLength).Concat(FastPathShortLength).ToArray();
            var stream = new ChunkedReadStream(bytes);
            var framed = new Framed<ChunkedReadStream>(stream);

            True(!framed.TryReadBufferedPdu(out _), "nothing buffered before the first read");

            var (_, first) = framed.ReadPdu().GetAwaiter().GetResult();
            True(first.SequenceEqual(TpktX224ConfirmSelectingTls), "first PDU bytes");
            Equal(1, stream.ReadCount, "one read buffered everything");

            True(framed.TryReadBufferedPdu(out var second), "second PDU is buffered");
            True(second.Payload.SequenceEqual(FastPathLongLength), "second PDU bytes");
            True(framed.TryReadBufferedPdu(out var third), "third PDU is buffered");
            True(third.Payload.SequenceEqual(FastPathShortLength), "third PDU bytes");
            True(!framed.TryReadBufferedPdu(out _), "nothing left");
            Equal(1, stream.ReadCount, "buffered PDUs did not read the stream");
        });

        // oracle: spec MS-RDPBCGR §2.2.9.1.1 (TPKT length counts the header, so 19 + 6 bytes hold one PDU and part of the next)
        yield return ("framed leaves a partial PDU buffered", () =>
        {
            var bytes = TpktX224ConfirmSelectingTls.Concat(TpktX224ConfirmWithoutNegotiation).ToArray();
            var stream = new ChunkedReadStream(bytes, TpktX224ConfirmSelectingTls.Length + 6);
            var framed = new Framed<ChunkedReadStream>(stream);

            framed.ReadPdu().GetAwaiter().GetResult();
            True(!framed.TryReadBufferedPdu(out _), "a partial PDU is not buffered");
            Equal(6, framed.Peek().Length, "partial PDU bytes stay buffered");

            var (_, second) = framed.ReadPdu().GetAwaiter().GetResult();
            True(second.SequenceEqual(TpktX224ConfirmWithoutNegotiation), "second PDU bytes");
        });

        // oracle: spec MS-RDPBCGR §2.2.9.1.1 and §2.2.9.1.2 for the PDU boundaries; mutant for the read count
        // (1 + 6 + 20 bytes, then the 3 left), which pins that no read is redundant
        yield return ("framed sync read splits PDUs across reads", () =>
        {
            var bytes = FastPathLongLength.Concat(TpktX224ConfirmSelectingTls).Concat(FastPathShortLength).ToArray();
            var stream = new ChunkedReadStream(bytes, 1, 6, 20);
            var framed = new Framed<ChunkedReadStream>(stream);

            var first = framed.ReadPduSync();
            True(first.Payload.SequenceEqual(FastPathLongLength), "first PDU bytes");
            var second = framed.ReadPduSync();
            True(second.Payload.SequenceEqual(TpktX224ConfirmSelectingTls), "second PDU bytes");
            var third = framed.ReadPduSync();
            True(third.Payload.SequenceEqual(FastPathShortLength), "third PDU bytes");
            Equal(4, stream.ReadCount, "reads");
            True(framed.Peek().IsEmpty, "nothing left in the buffer");

            var error = Throws<IronRdpLibException>(() => framed.ReadPduSync());
            Equal(IronRdpLibExceptionType.EndOfFile, error.ErrorType, "EOF error type");
        });

        // oracle: Framed remarks (a stream ending inside a PDU body is EndOfFile, like one ending between PDUs)
        yield return ("framed reports a stream that ends inside a PDU body as EndOfFile", () =>
        {
            var truncated = TpktX224ConfirmSelectingTls[..^3];

            var asyncFramed = new Framed<ChunkedReadStream>(new ChunkedReadStream(truncated));
            var asyncError = Throws<IronRdpLibException>(() => asyncFramed.ReadPdu().GetAwaiter().GetResult());
            Equal(IronRdpLibExceptionType.EndOfFile, asyncError.ErrorType, "async EOF error type");

            var syncFramed = new Framed<ChunkedReadStream>(new ChunkedReadStream(truncated));
            var syncError = Throws<IronRdpLibException>(() => syncFramed.ReadPduSync());
            Equal(IronRdpLibExceptionType.EndOfFile, syncError.ErrorType, "sync EOF error type");

            var hintFramed = new Framed<ChunkedReadStream>(new ChunkedReadStream(truncated));
            var hintError = Throws<IronRdpLibException>(() => hintFramed.ReadExact((nuint)TpktX224ConfirmSelectingTls.Length).GetAwaiter().GetResult());
            Equal(IronRdpLibExceptionType.EndOfFile, hintError.ErrorType, "ReadExact EOF error type");
        });

        // oracle: Framed remarks (a cancelled read leaves the stream read in flight and the next read finishes it);
        // mutant for the ReadCount assertions, which pin that the in-flight read is finished rather than reissued
        yield return ("framed read pdu stops when cancelled and the next read finishes the pending stream read", () =>
        {
            var stream = new BlockingReadStream(TpktX224ConfirmSelectingTls[..6]);
            var framed = new Framed<BlockingReadStream>(stream);
            using var cancellation = new CancellationTokenSource();

            var read = framed.ReadPdu(cancellation.Token);
            True(!read.IsCompleted, "read waits for the stream");
            Equal(2, stream.ReadCount, "the first six bytes and the blocked read");

            cancellation.Cancel();
            Throws<OperationCanceledException>(() => read.GetAwaiter().GetResult());
            Equal(6, framed.Peek().Length, "bytes read before the cancellation stay buffered");

            // The abandoned stream read completes only after the next read has started waiting on it.
            var next = framed.ReadPdu();
            True(!next.IsCompleted, "next read waits for the pending stream read");
            Equal(2, stream.ReadCount, "next read does not reissue the stream read");
            stream.Unblock(TpktX224ConfirmSelectingTls[6..].Concat(TpktX224ConfirmWithoutNegotiation).ToArray());
            var (_, pdu) = next.GetAwaiter().GetResult();
            True(pdu.SequenceEqual(TpktX224ConfirmSelectingTls), "next read completes the PDU in order");

            True(framed.TryReadBufferedPdu(out var buffered), "the second PDU arrived in the same stream read");
            True(buffered.Payload.SequenceEqual(TpktX224ConfirmWithoutNegotiation), "second PDU bytes");

            // Same handoff into the synchronous read.
            var cancelled = framed.ReadPdu(new CancellationToken(canceled: true));
            Throws<OperationCanceledException>(() => cancelled.GetAwaiter().GetResult());
            Equal(3, stream.ReadCount, "the cancelled read left a stream read in flight");
            var unblock = new Thread(() =>
            {
                Thread.Sleep(50);
                stream.Unblock(FastPathShortLength);
            });
            unblock.Start();
            var sync = framed.ReadPduSync();
            unblock.Join();
            True(sync.Payload.SequenceEqual(FastPathShortLength), "sync read finishes the pending stream read");
            Equal(3, stream.ReadCount, "one stream read per blocked wait");
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

    private static readonly byte[][] HandshakeServerPdus =
    {
        TpktX224ConfirmSelectingTls,
        // MCS Connect Response with SKIP_CHANNELJOIN_SUPPORTED (MS-RDPBCGR §2.2.1.4).
        new byte[]
        {
            0x03, 0x00, 0x00, 0x68, 0x02, 0xf0, 0x80, 0x7f, 0x66, 0x5e, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00,
            0x30, 0x1a, 0x02, 0x01, 0x22, 0x02, 0x01, 0x03, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01,
            0x00, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0xf8, 0x02, 0x01, 0x02, 0x04, 0x3a, 0x00, 0x05,
            0x00, 0x14, 0x7c, 0x00, 0x01, 0x33, 0x14, 0x76, 0x0a, 0x01, 0x01, 0x00, 0x01, 0xc0, 0x00, 0x4d,
            0x63, 0x44, 0x6e, 0x24, 0x01, 0x0c, 0x10, 0x00, 0x04, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x00, 0x00, 0x03, 0x0c, 0x08, 0x00, 0xeb, 0x03, 0x00, 0x00, 0x02, 0x0c, 0x0c, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        },
        // MCS Attach User Confirm for channel 1007 (MS-RDPBCGR §2.2.1.6).
        new byte[] { 0x03, 0x00, 0x00, 0x0b, 0x02, 0xf0, 0x80, 0x2e, 0x00, 0x00, 0x06 },
        // Licensing Error Message STATUS_VALID_CLIENT (MS-RDPBCGR §2.2.1.12).
        new byte[]
        {
            0x03, 0x00, 0x00, 0x22, 0x02, 0xf0, 0x80, 0x68, 0x00, 0x01, 0x03, 0xeb, 0x70, 0x14, 0x80, 0x00,
            0x00, 0x00, 0xff, 0x03, 0x14, 0x00, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00,
            0x00, 0x00,
        },
        // Server Demand Active with zero capability sets (MS-RDPBCGR §2.2.1.13.1).
        new byte[]
        {
            0x03, 0x00, 0x00, 0x24, 0x02, 0xf0, 0x80, 0x68, 0x00, 0x01, 0x03, 0xeb, 0x70, 0x16, 0x16, 0x00,
            0x11, 0x00, 0xea, 0x03, 0xea, 0x03, 0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        },
        // Server Font Map PDU (MS-RDPBCGR §2.2.1.22).
        new byte[]
        {
            0x03, 0x00, 0x00, 0x28, 0x02, 0xf0, 0x80, 0x68, 0x00, 0x01, 0x03, 0xeb, 0x70, 0x1a, 0x1a, 0x00,
            0x17, 0x00, 0xea, 0x03, 0xea, 0x03, 0x01, 0x00, 0x00, 0x02, 0x08, 0x00, 0x28, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00,
        },
    };

    private static (ActiveStage Stage, DecodedImage Image) ConnectedStage(ushort width, ushort height)
    {
        var connector = ClientConnector.New(TlsConfig(), "127.0.0.1:33890");
        var buf = WriteBuf.New();
        int i = 0;
        for (int steps = 0; !connector.DynState.IsTerminal(); steps++)
        {
            if (steps == 100)
            {
                throw new InvalidOperationException($"connector did not terminate after {i} server PDUs and {steps} steps");
            }

            buf.Clear();
            if (connector.ShouldPerformSecurityUpgrade())
            {
                connector.MarkSecurityUpgradeAsDone();
            }
            else if (connector.NextPduHint() is null)
            {
                connector.StepNoInput(buf);
            }
            else if (i == HandshakeServerPdus.Length)
            {
                throw new InvalidOperationException($"connector wants more than the {i} server PDUs in HandshakeServerPdus");
            }
            else
            {
                connector.Step(HandshakeServerPdus[i++], buf);
            }
        }

        Equal(HandshakeServerPdus.Length, i, "server PDUs consumed by the handshake");
        var stage = ActiveStage.New(connector.ConsumeAndCastToClientConnectorState().GetConnectedResult());
        return (stage, DecodedImage.New(PixelFormat.BgrX32, width, height));
    }

    private static byte SurfaceByte(int width, int x, int y) => (byte)(y * width * 4 + x + 1);

    // Sends one Fast-Path SetSurfaceBits (CODEC_ID_NONE, 32 bpp, bottom-up rows, MS-RDPBCGR §2.2.9.2.1)
    // and returns the GraphicsUpdate rectangle IronRDP reports for it.
    private static InclusiveRectangle PaintSurface(ActiveStage stage, DecodedImage image, ushort left, ushort top, ushort width, ushort height)
    {
        int dataLength = width * height * 4;
        var frame = new byte[28 + dataLength];
        int right = left + width;
        int bottom = top + height;
        new byte[]
        {
            0x00, (byte)(0x80 | frame.Length >> 8), (byte)frame.Length, 0x04, (byte)(22 + dataLength), (byte)(22 + dataLength >> 8),
            0x01, 0x00, (byte)left, (byte)(left >> 8), (byte)top, (byte)(top >> 8), (byte)right, (byte)(right >> 8), (byte)bottom, (byte)(bottom >> 8),
            0x20, 0x00, 0x00, 0x00, (byte)width, (byte)(width >> 8), (byte)height, (byte)(height >> 8),
            (byte)dataLength, (byte)(dataLength >> 8), (byte)(dataLength >> 16), (byte)(dataLength >> 24),
        }.CopyTo(frame, 0);
        for (int y = 0; y < height; y++)
        {
            for (int x = 0; x < width * 4; x++)
            {
                frame[28 + (height - 1 - y) * width * 4 + x] = SurfaceByte(width, x, y);
            }
        }

        var action = IronRdpPdu.New().FindSize(frame)!.Action;
        var outputs = stage.Process(image, action, frame);
        while (outputs.Next() is { } output)
        {
            if (output.EnumType == ActiveStageOutputType.GraphicsUpdate)
            {
                return output.GraphicsUpdate;
            }
        }

        throw new InvalidOperationException("SetSurfaceBits produced no GraphicsUpdate");
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

    private sealed class ChunkedReadStream : Stream
    {
        private readonly byte[] _bytes;
        private readonly Queue<int> _chunkSizes;
        private int _position;

        public ChunkedReadStream(byte[] bytes, params int[] chunkSizes)
        {
            _bytes = bytes;
            _chunkSizes = new Queue<int>(chunkSizes);
        }

        public int ReadCount { get; private set; }

        public override ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            return new ValueTask<int>(Read(buffer.Span));
        }

        public override int Read(byte[] buffer, int offset, int count)
        {
            return Read(buffer.AsSpan(offset, count));
        }

        public override int Read(Span<byte> buffer)
        {
            ReadCount++;
            var count = _chunkSizes.Count > 0 ? Math.Min(buffer.Length, _chunkSizes.Dequeue()) : buffer.Length;
            count = Math.Min(count, _bytes.Length - _position);
            _bytes.AsSpan(_position, count).CopyTo(buffer);
            _position += count;
            return count;
        }

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => _bytes.Length;
        public override long Position { get => _position; set => throw new NotSupportedException(); }
        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }

    // Serves the bytes it holds, then blocks until Unblock supplies more.
    private sealed class BlockingReadStream : MemoryStream
    {
        private TaskCompletionSource<byte[]> _more = new();

        public BlockingReadStream(byte[] first)
        {
            Write(first);
            Position = 0;
        }

        public int ReadCount { get; private set; }

        public void Unblock(byte[] bytes)
        {
            _more.SetResult(bytes);
        }

        public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            ReadCount++;
            if (Position == Length)
            {
                var bytes = await _more.Task.WaitAsync(cancellationToken);
                _more = new TaskCompletionSource<byte[]>();
                SetLength(0);
                Write(bytes);
                Position = 0;
            }

            return Read(buffer.Span);
        }
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
