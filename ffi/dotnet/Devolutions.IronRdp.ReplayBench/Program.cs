using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;
using Devolutions.IronRdp;

namespace Devolutions.IronRdp.ReplayBench;

/// <summary>
/// Deterministic replay benchmark + correctness gate for an IRDPREC1 capture, driving the real
/// .NET decode path (the one RDM uses) via the Diplomat FFI. Replays the recorded active-session
/// byte stream through a fresh ActiveStage per iteration and verifies the framebuffer CRC32 against
/// the recorded ground truth. See docs/plans/2026-06-03-ironrdp-benchmark-design.md.
/// </summary>
internal static class Program
{
    static async Task<int> Main(string[] args)
    {
        string? input = null;
        int iterations = 1;
        int warmup = 1;
        string? json = null;

        for (int i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--input": input = args[++i]; break;
                case "--iterations": iterations = int.Parse(args[++i]); break;
                case "--warmup": warmup = int.Parse(args[++i]); break;
                case "--json": json = args[++i]; break;
                default: Console.Error.WriteLine($"Unknown arg: {args[i]}"); return 2;
            }
        }

        if (input is null)
        {
            Console.Error.WriteLine("Usage: ReplayBench --input <capture.irdprec> [--iterations N] [--warmup K] [--json out.json]");
            return 2;
        }

        var manifestPath = Path.ChangeExtension(input, ".json");
        var checksumPath = Path.ChangeExtension(input, ".checksum.json");

        var jsonOpts = new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
        var capture = await File.ReadAllBytesAsync(input);
        var manifest = JsonSerializer.Deserialize<SessionManifest>(await File.ReadAllTextAsync(manifestPath), jsonOpts)
                       ?? throw new InvalidOperationException("failed to parse manifest");
        var checksum = JsonSerializer.Deserialize<ChecksumFile>(await File.ReadAllTextAsync(checksumPath), jsonOpts)
                       ?? throw new InvalidOperationException("failed to parse checksum");

        Console.WriteLine($"Replaying {input} ({capture.Length} bytes, {manifest.DesktopWidth}x{manifest.DesktopHeight}) — {warmup} warmup + {iterations} measured iterations");

        for (int i = 0; i < warmup; i++)
        {
            await RunPass(capture, manifest);
        }

        var timingsMs = new List<double>();
        uint lastCrc = 0;
        ulong pdus = 0;
        for (int i = 0; i < Math.Max(1, iterations); i++)
        {
            var (elapsedMs, crc, n) = await RunPass(capture, manifest);
            timingsMs.Add(elapsedMs);
            lastCrc = crc;
            pdus = n;
        }

        var canonical = lastCrc.ToString("x8");
        var matches = canonical == checksum.Crc32;

        double min = timingsMs.Min();
        double max = timingsMs.Max();
        double med = Median(timingsMs);

        Console.WriteLine($"PDUs: {pdus} | decode ms: min={min:F3} median={med:F3} max={max:F3}");
        Console.WriteLine($"checksum: replay={canonical} expected={checksum.Crc32} -> {(matches ? "MATCH" : "MISMATCH")}");

        if (json is not null)
        {
            var result = new ResultJson
            {
                SchemaVersion = 1,
                Capture = input,
                Frontend = "dotnet",
                Iterations = iterations,
                Warmup = warmup,
                Pdus = pdus,
                DecodeMsMin = min,
                DecodeMsMedian = med,
                DecodeMsMax = max,
                CanonicalChecksum = canonical,
                ExpectedChecksum = checksum.Crc32,
                MatchesGroundTruth = matches,
            };
            await File.WriteAllTextAsync(json, JsonSerializer.Serialize(result, new JsonSerializerOptions { WriteIndented = true }));
            Console.WriteLine($"Wrote {json}");
        }

        return matches ? 0 : 1;
    }

    /// One replay pass: fresh session, replay the whole capture, return (decode ms, framebuffer CRC32, PDU count).
    static async Task<(double, uint, ulong)> RunPass(byte[] capture, SessionManifest manifest)
    {
        using var builder = ReplayConnectionBuilder.New(
            manifest.IoChannelId,
            manifest.UserChannelId,
            manifest.ShareId,
            manifest.DesktopWidth,
            manifest.DesktopHeight,
            manifest.EnableServerPointer,
            manifest.PointerSoftwareRendering);
        builder.SetCompression(manifest.CompressionType ?? string.Empty);
        foreach (var ch in manifest.Channels)
        {
            builder.AddChannel(ch.Name, ch.Id);
        }

        using var connectionResult = builder.Build();
        using var image = DecodedImage.New(PixelFormat.RgbA32, manifest.DesktopWidth, manifest.DesktopHeight);
        using var activeStage = ActiveStage.New(connectionResult);
        var framed = new Framed<ReplayStream>(new ReplayStream(capture));

        ulong pdus = 0;
        var sw = Stopwatch.StartNew();
        var done = false;
        while (!done)
        {
            Devolutions.IronRdp.Action action;
            byte[] payload;
            try
            {
                (action, payload) = await framed.ReadPdu();
            }
            catch (IronRdpLibException e) when (e.ErrorType == IronRdpLibExceptionType.EndOfFile)
            {
                break; // clean end of capture
            }

            using var outputs = activeStage.Process(image, action, payload);
            pdus++;

            while (!outputs.IsEmpty())
            {
                using var output = outputs.Next()!;
                if (output.GetEnumType() == ActiveStageOutputType.Terminate)
                {
                    done = true;
                }
                // Response frames are dropped: there is no server on replay.
            }
        }
        sw.Stop();

        var crc = FramebufferCrc32(image);
        return (sw.Elapsed.TotalMilliseconds, crc, pdus);
    }

    /// CRC32 over the canonical framebuffer (RGBA with alpha masked to 0xFF). MUST match
    /// `ironrdp_replay_core::framebuffer_crc32` / `ironrdp_client::record::framebuffer_crc32`.
    static uint FramebufferCrc32(DecodedImage image)
    {
        var slice = image.GetData();
        var buf = new byte[(int)slice.GetSize()];
        slice.Fill(buf);

        uint crc = 0xFFFFFFFF;
        var pixel = new byte[4];
        for (int i = 0; i + 4 <= buf.Length; i += 4)
        {
            pixel[0] = buf[i];
            pixel[1] = buf[i + 1];
            pixel[2] = buf[i + 2];
            pixel[3] = 0xFF;
            for (int b = 0; b < 4; b++)
            {
                crc ^= pixel[b];
                for (int k = 0; k < 8; k++)
                {
                    var mask = (uint)(-(int)(crc & 1));
                    crc = (crc >> 1) ^ (0xEDB88320 & mask);
                }
            }
        }
        return crc ^ 0xFFFFFFFF;
    }

    static double Median(List<double> v)
    {
        if (v.Count == 0) return 0;
        var s = v.OrderBy(x => x).ToList();
        int n = s.Count;
        return n % 2 == 1 ? s[n / 2] : (s[n / 2 - 1] + s[n / 2]) / 2.0;
    }
}

/// In-memory replay transport: serves the recorded capture bytes; discards writes.
internal sealed class ReplayStream : Stream
{
    private readonly byte[] _data;
    private int _pos;

    public ReplayStream(byte[] data) => _data = data;

    public override bool CanRead => true;
    public override bool CanWrite => true;
    public override bool CanSeek => false;
    public override long Length => _data.Length;
    public override long Position { get => _pos; set => throw new NotSupportedException(); }

    public override int Read(byte[] buffer, int offset, int count)
    {
        int n = Math.Min(count, _data.Length - _pos);
        if (n <= 0) return 0; // EOF
        System.Array.Copy(_data, _pos, buffer, offset, n);
        _pos += n;
        return n;
    }

    public override void Write(byte[] buffer, int offset, int count) { /* discard */ }
    public override void Flush() { }
    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
    public override void SetLength(long value) => throw new NotSupportedException();
}

internal sealed class ChannelManifest
{
    public string Name { get; set; } = string.Empty;
    public ushort Id { get; set; }
}

internal sealed class SessionManifest
{
    public ushort DesktopWidth { get; set; }
    public ushort DesktopHeight { get; set; }
    public ushort IoChannelId { get; set; }
    public ushort UserChannelId { get; set; }
    public uint ShareId { get; set; }
    public string? CompressionType { get; set; }
    public bool EnableServerPointer { get; set; }
    public bool PointerSoftwareRendering { get; set; }
    public List<ChannelManifest> Channels { get; set; } = new();
}

internal sealed class ChecksumFile
{
    public string Crc32 { get; set; } = string.Empty;
}

internal sealed class ResultJson
{
    [JsonPropertyName("schema_version")] public int SchemaVersion { get; set; }
    [JsonPropertyName("capture")] public string Capture { get; set; } = string.Empty;
    [JsonPropertyName("frontend")] public string Frontend { get; set; } = string.Empty;
    [JsonPropertyName("iterations")] public int Iterations { get; set; }
    [JsonPropertyName("warmup")] public int Warmup { get; set; }
    [JsonPropertyName("pdus")] public ulong Pdus { get; set; }
    [JsonPropertyName("decode_ms_min")] public double DecodeMsMin { get; set; }
    [JsonPropertyName("decode_ms_median")] public double DecodeMsMedian { get; set; }
    [JsonPropertyName("decode_ms_max")] public double DecodeMsMax { get; set; }
    [JsonPropertyName("canonical_checksum")] public string CanonicalChecksum { get; set; } = string.Empty;
    [JsonPropertyName("expected_checksum")] public string ExpectedChecksum { get; set; } = string.Empty;
    [JsonPropertyName("matches_ground_truth")] public bool MatchesGroundTruth { get; set; }
}
