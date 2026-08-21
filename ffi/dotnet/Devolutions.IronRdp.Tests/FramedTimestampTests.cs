using System.Text;

using Xunit;

namespace Devolutions.IronRdp.Tests;

/// <summary>
/// The arrival time a sequence receives must be when the bytes were read off the socket, not when
/// the managed caller got around to stepping the sequence.
/// </summary>
public class FramedTimestampTests
{
    private static readonly byte[] PduA = { 1, 2, 3, 4 };
    private static readonly byte[] PduB = { 5, 6, 7, 8 };

    /// <summary>
    /// How long tests wait to make a "then" clearly distinguishable from a "now".
    /// </summary>
    private const int GapMs = 60;

    /// <summary>
    /// Timer granularity and scheduling make the observed gap shorter than the requested one.
    /// </summary>
    private const ulong MinObservedGapMs = 30;

    [Fact]
    public void MonotonicInstantIsANonOwningValue()
    {
        Assert.True(typeof(MonotonicInstant).IsValueType);
        Assert.False(typeof(IDisposable).IsAssignableFrom(typeof(MonotonicInstant)));
        Assert.Null(typeof(MonotonicInstant).GetConstructor(new[] { typeof(ulong) }));
        Assert.Equal(sizeof(ulong), System.Runtime.InteropServices.Marshal.SizeOf<MonotonicInstant>());
    }

    [Fact]
    public async Task ZeroSizeHintFailsBeforeReading()
    {
        var stream = new ScriptedStream(PduA);
        var framed = new Framed<ScriptedStream>(stream);

        await Assert.ThrowsAsync<InvalidDataException>(async () => await framed.ReadByHint(new FixedSizeHint(0)));

        Assert.Equal(0, stream.ReadCount);
    }

    [Fact]
    public async Task TwoPdusFromOneReadShareTheirArrivalTime()
    {
        var stream = new ScriptedStream(Concat(PduA, PduB));
        var framed = new Framed<ScriptedStream>(stream);
        var hint = new FixedSizeHint(PduA.Length);

        var (first, firstAt) = await framed.ReadByHint(hint);
        var (second, secondAt) = await framed.ReadByHint(hint);

        Assert.Equal(PduA, first);
        Assert.Equal(PduB, second);
        Assert.Equal(1, stream.ReadCount);
        AssertSameInstant(firstAt, secondAt);
    }

    [Fact]
    public async Task SeparateReadsAdvanceTheArrivalTime()
    {
        var stream = new ScriptedStream(PduA, PduB) { DelayBeforeChunk = TimeSpan.FromMilliseconds(GapMs) };
        var framed = new Framed<ScriptedStream>(stream);
        var hint = new FixedSizeHint(PduA.Length);

        var (_, firstAt) = await framed.ReadByHint(hint);
        var (_, secondAt) = await framed.ReadByHint(hint);

        Assert.Equal(2, stream.ReadCount);
        Assert.True(secondAt.MillisSince(firstAt) >= MinObservedGapMs);
    }

    [Fact]
    public async Task ABufferedPduKeepsTheArrivalTimeOfItsRead()
    {
        var stream = new ScriptedStream(Concat(PduA, PduB));
        var framed = new Framed<ScriptedStream>(stream);
        var hint = new FixedSizeHint(PduA.Length);

        var (_, firstAt) = await framed.ReadByHint(hint);
        await Task.Delay(GapMs);
        var (_, secondAt) = await framed.ReadByHint(hint);

        Assert.Equal(1, stream.ReadCount);
        AssertSameInstant(firstAt, secondAt);
        Assert.True(MonotonicInstant.Now().MillisSince(secondAt) >= MinObservedGapMs);
    }

    [Fact]
    public async Task LeftoverCarriesTheArrivalTimeIntoANewFramed()
    {
        var stream = new ScriptedStream(Concat(PduA, PduB));
        var framed = new Framed<ScriptedStream>(stream);
        var hint = new FixedSizeHint(PduA.Length);

        var (_, firstAt) = await framed.ReadByHint(hint);

        var (inner, leftover) = framed.GetInner();
        Assert.False(leftover.IsEmpty);
        var rebuilt = new Framed<ScriptedStream>(inner, leftover);

        await Task.Delay(GapMs);
        var (second, secondAt) = await rebuilt.ReadByHint(hint);

        Assert.Equal(PduB, second);
        Assert.Equal(1, stream.ReadCount);
        AssertSameInstant(firstAt, secondAt);
    }

    [Fact]
    public async Task SingleSequenceStepPassesTheReadTimeNotTheCallTime()
    {
        // A TPKT frame announcing 11 bytes, which is all the X.224 hint looks at. The payload does
        // not have to be a valid connection confirm: the sequence records the arrival time before
        // it ever tries to decode.
        byte[] x224Framed = { 0x03, 0x00, 0x00, 0x0B, 0x06, 0xD0, 0x00, 0x00, 0x12, 0x34, 0x00 };

        var stream = new ScriptedStream(Concat(PduA, x224Framed));
        var framed = new Framed<ScriptedStream>(stream);

        // Drain a first PDU so that the X.224 one stays buffered from that very same read.
        var (_, readAt) = await framed.ReadByHint(new FixedSizeHint(PduA.Length));
        Assert.Equal(1, stream.ReadCount);

        var connector = ClientConnector.New(BuildConfig(), "127.0.0.1:1234");
        var writeBuf = WriteBuf.New();
        connector.StepNoInput(writeBuf); // Sends the connection request, so X.224 is what comes next.

        var recording = new RecordingSequence(connector);
        Assert.NotNull(recording.NextPduHint());

        await Task.Delay(GapMs);

        try
        {
            await Connection.SingleSequenceStep(recording, writeBuf, framed);
        }
        catch (IronRdpException)
        {
            // Expected: the payload is not a real connection confirm.
        }

        Assert.Equal(1, stream.ReadCount);
        Assert.True(recording.ReceivedAt.HasValue);
        Assert.True(recording.StepEnteredAt.HasValue);
        AssertSameInstant(readAt, recording.ReceivedAt.Value);
        Assert.True(recording.StepEnteredAt.Value.MillisSince(recording.ReceivedAt.Value) >= MinObservedGapMs);
    }

    private static void AssertSameInstant(MonotonicInstant left, MonotonicInstant right)
    {
        Assert.Equal(0ul, left.MillisSince(right));
        Assert.Equal(0ul, right.MillisSince(left));
    }

    private static byte[] Concat(params byte[][] chunks)
    {
        return chunks.SelectMany(chunk => chunk).ToArray();
    }

    private static Config BuildConfig()
    {
        var builder = ConfigBuilder.New();
        builder.WithUsernameAndPassword("user", "password");
        builder.SetDomain("domain");
        builder.SetDesktopSize(1080, 1920);
        builder.SetClientName("IronRdpTests");
        builder.SetClientDir("C:\\");
        builder.SetPerformanceFlags(PerformanceFlags.NewDefault());
        return builder.Build();
    }
}

/// <summary>
/// Wraps a real sequence to record what <c>SingleSequenceStep</c> hands it.
/// </summary>
internal sealed class RecordingSequence : ISequence
{
    private readonly ClientConnector _inner;

    public MonotonicInstant? ReceivedAt { get; private set; }

    public MonotonicInstant? StepEnteredAt { get; private set; }

    public RecordingSequence(ClientConnector inner)
    {
        _inner = inner;
    }

    public PduHint? NextPduHint()
    {
        return _inner.NextPduHint();
    }

    public Written Step(byte[] pdu, MonotonicInstant receivedAt, WriteBuf buf)
    {
        ReceivedAt = receivedAt;
        StepEnteredAt = MonotonicInstant.Now();
        return _inner.Step(pdu, receivedAt, buf);
    }

    public Written StepNoInput(WriteBuf buf)
    {
        return _inner.StepNoInput(buf);
    }
}

/// <summary>
/// Serves a fixed list of chunks, one per read, so tests decide exactly how bytes are split.
/// </summary>
internal sealed class ScriptedStream : Stream
{
    private readonly Queue<byte[]> _chunks;

    public int ReadCount { get; private set; }

    public TimeSpan DelayBeforeChunk { get; init; } = TimeSpan.Zero;

    public ScriptedStream(params byte[][] chunks)
    {
        _chunks = new Queue<byte[]>(chunks);
    }

    public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
    {
        if (_chunks.Count == 0)
        {
            return 0;
        }

        if (DelayBeforeChunk > TimeSpan.Zero)
        {
            await Task.Delay(DelayBeforeChunk, cancellationToken);
        }

        var chunk = _chunks.Dequeue();
        chunk.CopyTo(buffer.Span);
        ReadCount++;

        return chunk.Length;
    }

    public override ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default)
    {
        return ValueTask.CompletedTask;
    }

    public override bool CanRead => true;

    public override bool CanSeek => false;

    public override bool CanWrite => true;

    public override long Length => throw new NotSupportedException();

    public override long Position
    {
        get => throw new NotSupportedException();
        set => throw new NotSupportedException();
    }

    public override void Flush()
    {
    }

    public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

    public override void SetLength(long value) => throw new NotSupportedException();

    public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
}

/// <summary>
/// Reports a PDU as soon as the buffer holds the expected number of bytes.
/// </summary>
internal sealed class FixedSizeHint : IPduHint
{
    private readonly int _size;

    public FixedSizeHint(int size)
    {
        _size = size;
    }

    public (bool, int)? FindSize(byte[] bytes)
    {
        return bytes.Length >= _size ? (true, _size) : null;
    }
}
