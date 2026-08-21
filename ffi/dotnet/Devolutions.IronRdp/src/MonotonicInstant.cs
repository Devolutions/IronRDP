using System.Diagnostics;

namespace Devolutions.IronRdp;

/// <summary>
/// A reading of the managed driver's monotonic clock.
/// </summary>
public readonly struct MonotonicInstant
{
    private static readonly Stopwatch Clock = Stopwatch.StartNew();

    private readonly ulong _milliseconds;

    private MonotonicInstant(ulong milliseconds)
    {
        _milliseconds = milliseconds;
    }

    internal ulong Milliseconds => _milliseconds;

    /// <summary>
    /// Reads the clock now.
    /// </summary>
    public static MonotonicInstant Now()
    {
        return new MonotonicInstant((ulong)Clock.ElapsedMilliseconds);
    }

    /// <summary>
    /// Returns the elapsed milliseconds since <paramref name="earlier"/>, saturating at zero.
    /// </summary>
    public ulong MillisSince(MonotonicInstant earlier)
    {
        return _milliseconds >= earlier._milliseconds ? _milliseconds - earlier._milliseconds : 0;
    }
}
