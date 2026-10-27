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

        // oracle: mutant source and destination row offsets, inclusive bounds, and vertical rectangle guards
        yield return ("decoded image copies only the requested rectangle", () =>
        {
            var image = NonUniformImage();
            const int stride = 8 * 4 + 4;
            var buffer = new byte[stride * 4];
            Array.Fill(buffer, (byte)0xff);

            image.CopyRectInto(InclusiveRectangle.New(2, 1, 4, 2), buffer, stride);

            for (int y = 0; y < 4; y++)
            {
                for (int x = 0; x < stride; x++)
                {
                    bool inside = y is >= 1 and <= 2 && x >= 2 * 4 && x < 5 * 4;
                    byte expected = inside ? (byte)(y * 8 * 4 + x + 1) : (byte)0xff;
                    Equal(expected, buffer[y * stride + x], $"byte ({x}, {y})");
                }
            }

            var inverted = Throws<IronRdpException>(() => InclusiveRectangle.New(3, 0, 2, 0));
            True(inverted.Message.Contains("rectangle is inverted"), $"error message: {inverted.Message}");
            var invertedVertical = Throws<IronRdpException>(() => InclusiveRectangle.New(0, 3, 0, 2));
            True(invertedVertical.Message.Contains("rectangle is inverted"), $"error message: {invertedVertical.Message}");
            var tooLarge = Throws<IronRdpException>(() => InclusiveRectangle.New(0, 0, ushort.MaxValue, 0));
            True(tooLarge.Message.Contains("rectangle is too large"), $"error message: {tooLarge.Message}");
            var tooTall = Throws<IronRdpException>(() => InclusiveRectangle.New(0, 0, 0, ushort.MaxValue));
            True(tooTall.Message.Contains("rectangle is too large"), $"error message: {tooTall.Message}");
            Equal((int)ushort.MaxValue, (int)InclusiveRectangle.New(1, 0, ushort.MaxValue, 0).Width, "widest rectangle");
            Equal((int)ushort.MaxValue, (int)InclusiveRectangle.New(0, 1, 0, ushort.MaxValue).Height, "tallest rectangle");
            var outside = Throws<IronRdpException>(() => image.CopyRectInto(InclusiveRectangle.New(0, 0, 8, 0), buffer, stride));
            True(outside.Message.Contains("rectangle is outside the image"), $"error message: {outside.Message}");
            var outsideVertical = Throws<IronRdpException>(() => image.CopyRectInto(InclusiveRectangle.New(0, 0, 0, 4), buffer, stride));
            True(outsideVertical.Message.Contains("rectangle is outside the image"), $"error message: {outsideVertical.Message}");
            Throws<IronRdpException>(() => image.CopyRectInto(InclusiveRectangle.New(0, 0, 7, 0), buffer, 8 * 4 - 1));
            Throws<IronRdpException>(() => image.CopyRectInto(InclusiveRectangle.New(0, 0, 7, 3), new byte[stride * 3], stride));
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

    private static DecodedImage NonUniformImage()
    {
        var connector = ClientConnector.New(TlsConfig(), "127.0.0.1:33890");
        var buf = WriteBuf.New();
        int i = 0;
        while (!connector.DynState.IsTerminal())
        {
            buf.Clear();
            if (connector.ShouldPerformSecurityUpgrade())
            {
                connector.MarkSecurityUpgradeAsDone();
            }
            else if (connector.NextPduHint() is null)
            {
                connector.StepNoInput(buf);
            }
            else
            {
                connector.Step(HandshakeServerPdus[i++], buf);
            }
        }

        var stage = ActiveStage.New(connector.ConsumeAndCastToClientConnectorState().GetConnectedResult());
        var image = DecodedImage.New(PixelFormat.BgrX32, 8, 4);
        var frame = new byte[28 + 8 * 4 * 4];
        // Fast-Path SetSurfaceBits (CODEC_ID_NONE, 32 bpp, 8x4, bottom-up rows, MS-RDPBCGR §2.2.9.2.1).
        new byte[]
        {
            0x00, 0x80, 0x9c, 0x04, 0x96, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x04, 0x00,
            0x20, 0x00, 0x00, 0x00, 0x08, 0x00, 0x04, 0x00, 0x80, 0x00, 0x00, 0x00,
        }.CopyTo(frame, 0);
        for (int y = 0; y < 4; y++)
        {
            for (int x = 0; x < 8 * 4; x++)
            {
                frame[28 + (3 - y) * 8 * 4 + x] = (byte)(y * 8 * 4 + x + 1);
            }
        }

        var action = IronRdpPdu.New().FindSize(frame)!.Action;
        stage.Process(image, action, frame);
        return image;
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
