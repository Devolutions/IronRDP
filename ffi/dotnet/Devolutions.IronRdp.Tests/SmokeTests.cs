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

        // oracle: spec MS-RDPBCGR §2.2.8.1.2 (TS_FP_INPUT_PDU) and §2.2.8.1.2.2.3 (TS_FP_POINTER_EVENT)
        yield return ("fast-path input encodes to the spec bytes without a stage", () =>
        {
            var events = InputDatabase.New().Apply(MousePosition.New(10, 20).AsMoveOperation());
            using var frame = events.Encode();
            var bytes = new byte[(int)frame.Size];
            frame.Fill(bytes);
            True(bytes.SequenceEqual(new byte[] { 0x04, 0x09, 0x20, 0x00, 0x08, 0x0a, 0x00, 0x14, 0x00 }), $"frame: {Convert.ToHexString(bytes)}");
        });

        // oracle: ffi/README.md says Encode() sends the same bytes as ProcessFastpathInput
        yield return ("fast-path input encodes the bytes the stage would send", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var events = InputDatabase.New().Apply(MouseButton.New(MouseButtonType.Left).AsOperationMouseButtonPressed());

            using var encoded = events.Encode();
            var encodedBytes = new byte[(int)encoded.Size];
            encoded.Fill(encodedBytes);

            var sent = new List<byte>();
            var outputs = stage.ProcessFastpathInput(image, events);
            while (outputs.Next() is { } output)
            {
                Equal(ActiveStageOutputType.ResponseFrame, output.EnumType, "output type");
                using var responseFrame = output.ResponseFrame;
                var responseBytes = new byte[(int)responseFrame.Size];
                responseFrame.Fill(responseBytes);
                sent.AddRange(responseBytes);
            }

            True(encodedBytes.Length > 0 && sent.SequenceEqual(encodedBytes), "same bytes on the wire");
        });

        // oracle: ffi/README.md says Encode() reads no ActiveStage or DecodedImage
        yield return ("fast-path input encodes after the stage and image are disposed", () =>
        {
            var (stage, image) = ConnectedStage(8, 4);
            var events = InputDatabase.New().Apply(WheelRotations.New(true, 120).AsOperation());
            stage.Dispose();
            image.Dispose();
            using var frame = events.Encode();
            True(frame.Size > 0, "frame is not empty");
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

        // oracle: spec MS-RDPEPS §2.2.1.1 (RDP_PRECONNECTION_PDU_V1), same vector as ironrdp-testsuite-core tests/pcb.rs
        yield return ("preconnection blob v1 encodes the id", () =>
        {
            var bytes = ToArray(PreconnectionBlob.NewV1(0xEEC699EB).Encode());
            True(bytes.SequenceEqual(new byte[]
            {
                0x10, 0x00, 0x00, 0x00, // cbSize = 16
                0x00, 0x00, 0x00, 0x00, // Flags = 0
                0x01, 0x00, 0x00, 0x00, // Version = 1
                0xeb, 0x99, 0xc6, 0xee, // Id = 0xEEC699EB
            }), $"encoded bytes: {Convert.ToHexString(bytes)}");
        });

        // oracle: spec MS-RDPEPS §2.2.1.2 (cchPCB counts the null terminator), same vector as ironrdp-testsuite-core tests/pcb.rs
        yield return ("preconnection blob v2 encodes a null-terminated UTF-16 payload", () =>
        {
            var bytes = ToArray(PreconnectionBlob.NewV2(0, "TestVM").Encode());
            True(bytes.SequenceEqual(new byte[]
            {
                0x20, 0x00, 0x00, 0x00, // cbSize = 32
                0x00, 0x00, 0x00, 0x00, // Flags = 0
                0x02, 0x00, 0x00, 0x00, // Version = 2
                0x00, 0x00, 0x00, 0x00, // Id = 0
                0x07, 0x00, // cchPCB = 7 characters, "TestVM" plus the null terminator
                0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x56, 0x00, 0x4d, 0x00, 0x00, 0x00, // wszPCB = "TestVM\0"
            }), $"encoded bytes: {Convert.ToHexString(bytes)}");
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

        yield return ("display control is unavailable without its dynamic channel", () =>
        {
            var (stage, _) = ConnectedStage(8, 4);
            Equal(DisplayControlState.Unavailable, stage.DisplayControlState, "state");
            True(stage.DisplayControlCapabilities is null, "no capabilities");
            True(stage.EncodedResize(1024, 768) is null, "no resize frame");
        });

        // oracle: spec MS-RDPEDISP §1.3 (the client sends a monitor layout only after the server's DISPLAYCONTROL_CAPS_PDU)
        // and §2.2.2.1 (MaxNumMonitors=2, MaxMonitorAreaFactorA=3840, MaxMonitorAreaFactorB=2400 in DisplayControlCapsPdu)
        yield return ("display control becomes ready when the server capabilities arrive", () =>
        {
            var (stage, image) = ConnectedStage(8, 4, displayControl: true);
            Equal(DisplayControlState.Pending, stage.DisplayControlState, "state before the channel opens");
            True(stage.EncodedResize(1024, 768) is null, "no resize frame before the channel opens");

            var createResponses = ProcessFrame(stage, image, DrdynvcDisplayControlCreateRequest);
            True(createResponses.Any(o => o.EnumType == ActiveStageOutputType.ResponseFrame), "create response is sent");
            Equal(DisplayControlState.Pending, stage.DisplayControlState, "state before the capabilities");
            True(stage.DisplayControlCapabilities is null, "no capabilities before the server sends them");
            True(stage.EncodedResize(1024, 768) is null, "no resize frame before the capabilities");

            ProcessFrame(stage, image, DisplayControlCapsPdu);
            Equal(DisplayControlState.Ready, stage.DisplayControlState, "state after the capabilities");
            var capabilities = stage.DisplayControlCapabilities;
            True(capabilities is not null, "capabilities after the server sends them");
            Equal(2u, capabilities!.MaxNumMonitors, "max monitors");
            Equal(2ul * 3840 * 2400, capabilities.MaxMonitorArea, "max monitor area");

            var resize = stage.EncodedResize(1024, 768);
            True(resize is not null, "resize frame after the capabilities");
            True(resize!.Next() is { EnumType: ActiveStageOutputType.ResponseFrame }, "resize is a response frame");
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

    // MCS Connect Response with SKIP_CHANNELJOIN_SUPPORTED and no static channels (MS-RDPBCGR §2.2.1.4).
    private static readonly byte[] McsConnectResponse =
    {
        0x03, 0x00, 0x00, 0x68, 0x02, 0xf0, 0x80, 0x7f, 0x66, 0x5e, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00,
        0x30, 0x1a, 0x02, 0x01, 0x22, 0x02, 0x01, 0x03, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01,
        0x00, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0xf8, 0x02, 0x01, 0x02, 0x04, 0x3a, 0x00, 0x05,
        0x00, 0x14, 0x7c, 0x00, 0x01, 0x33, 0x14, 0x76, 0x0a, 0x01, 0x01, 0x00, 0x01, 0xc0, 0x00, 0x4d,
        0x63, 0x44, 0x6e, 0x24, 0x01, 0x0c, 0x10, 0x00, 0x04, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x08, 0x00, 0x00, 0x00, 0x03, 0x0c, 0x08, 0x00, 0xeb, 0x03, 0x00, 0x00, 0x02, 0x0c, 0x0c, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    };

    private static readonly byte[][] HandshakeServerPdus =
    {
        TpktX224ConfirmSelectingTls,
        McsConnectResponse,
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

    // The same Connect Response with one static channel, 1004, for the drdynvc channel the client requests.
    private static readonly byte[] McsConnectResponseWithDrdynvc =
    {
        0x03, 0x00, 0x00, 0x6c, 0x02, 0xf0, 0x80, 0x7f, 0x66, 0x62, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00,
        0x30, 0x1a, 0x02, 0x01, 0x22, 0x02, 0x01, 0x03, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01, 0x02, 0x01,
        0x00, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0xf8, 0x02, 0x01, 0x02, 0x04, 0x3e, 0x00, 0x05,
        0x00, 0x14, 0x7c, 0x00, 0x01, 0x36, 0x14, 0x00, 0x01, 0x01, 0x01, 0x00, 0x01, 0xc0, 0x00, 0x4d,
        0x63, 0x44, 0x6e, 0x28, 0x01, 0x0c, 0x10, 0x00, 0x04, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x08, 0x00, 0x00, 0x00, 0x03, 0x0c, 0x0c, 0x00, 0xeb, 0x03, 0x01, 0x00, 0xec, 0x03, 0x00, 0x00,
        0x02, 0x0c, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    };

    // DRDYNVC Create Request for "Microsoft::Windows::RDS::DisplayControl" as channel 1 on static channel 1004
    // (MS-RDPEDYC §2.2.2.1), inside an MCS Send Data Indication.
    private static readonly byte[] DrdynvcDisplayControlCreateRequest =
    {
        0x03, 0x00, 0x00, 0x40, 0x02, 0xf0, 0x80, 0x68, 0x00, 0x01, 0x03, 0xec, 0x70, 0x32, 0x2a, 0x00,
        0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x10, 0x01, 0x4d, 0x69, 0x63, 0x72, 0x6f, 0x73, 0x6f, 0x66,
        0x74, 0x3a, 0x3a, 0x57, 0x69, 0x6e, 0x64, 0x6f, 0x77, 0x73, 0x3a, 0x3a, 0x52, 0x44, 0x53, 0x3a,
        0x3a, 0x44, 0x69, 0x73, 0x70, 0x6c, 0x61, 0x79, 0x43, 0x6f, 0x6e, 0x74, 0x72, 0x6f, 0x6c, 0x00,
    };

    // DISPLAYCONTROL_CAPS_PDU with MaxNumMonitors=2, MaxMonitorAreaFactorA=3840, MaxMonitorAreaFactorB=2400
    // (MS-RDPEDISP §2.2.2.1) as DRDYNVC data on channel 1.
    private static readonly byte[] DisplayControlCapsPdu =
    {
        0x03, 0x00, 0x00, 0x2c, 0x02, 0xf0, 0x80, 0x68, 0x00, 0x01, 0x03, 0xec, 0x70, 0x1e, 0x16, 0x00,
        0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x30, 0x01, 0x05, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x60, 0x09, 0x00, 0x00,
    };

    private static (ActiveStage Stage, DecodedImage Image) ConnectedStage(ushort width, ushort height, bool displayControl = false)
    {
        var connector = ClientConnector.New(TlsConfig(), "127.0.0.1:33890");
        var serverPdus = HandshakeServerPdus.ToArray();
        if (displayControl)
        {
            connector.WithDynamicChannelDisplayControl();
            serverPdus[1] = McsConnectResponseWithDrdynvc;
        }

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
            else if (i == serverPdus.Length)
            {
                throw new InvalidOperationException($"connector wants more than the {i} server PDUs in HandshakeServerPdus");
            }
            else
            {
                connector.Step(serverPdus[i++], buf);
            }
        }

        Equal(serverPdus.Length, i, "server PDUs consumed by the handshake");
        var stage = ActiveStage.New(connector.ConsumeAndCastToClientConnectorState().GetConnectedResult());
        return (stage, DecodedImage.New(PixelFormat.BgrX32, width, height));
    }

    private static List<ActiveStageOutput> ProcessFrame(ActiveStage stage, DecodedImage image, byte[] frame)
    {
        var action = IronRdpPdu.New().FindSize(frame)!.Action;
        var outputs = stage.Process(image, action, frame);
        var collected = new List<ActiveStageOutput>();
        while (outputs.Next() is { } output)
        {
            collected.Add(output);
        }

        return collected;
    }

    private static byte SurfaceByte(int width, int x, int y) => (byte)(y * width * 4 + x + 1);

    private static byte[] ToArray(VecU8 vec)
    {
        using (vec)
        {
            var bytes = new byte[(int)vec.Size];
            vec.Fill(bytes);
            return bytes;
        }
    }

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

    private sealed class ChunkedReadStream : MemoryStream
    {
        private readonly Queue<int> _chunkSizes;

        public ChunkedReadStream(byte[] buffer, params int[] chunkSizes) : base(buffer)
        {
            _chunkSizes = new Queue<int>(chunkSizes);
        }

        public override ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            var maxCount = _chunkSizes.Count > 0 ? Math.Min(buffer.Length, _chunkSizes.Dequeue()) : buffer.Length;
            return base.ReadAsync(buffer[..maxCount], cancellationToken);
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
