// Test guide: see tests/README.md for how to read these tests
// (fake timers, setup helpers, mock patterns, test.fails convention).

import { vi, describe, test, expect, beforeEach, afterEach } from 'vitest';
import { createReplayStore } from '../src/services/replay-store.svelte.js';
import { createMockDataSource, makePdu, makePdus } from './helpers/mock-data-source.js';
import { createMockWasmReplay } from './helpers/mock-wasm-replay.js';

let store: ReturnType<typeof createReplayStore>;
let ds: ReturnType<typeof createMockDataSource>;
let mock: ReturnType<typeof createMockWasmReplay>;

beforeEach(() => {
    vi.useFakeTimers({
        toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'setTimeout', 'performance'],
    });
    ds = createMockDataSource({ durationMs: 60_000 });
    mock = createMockWasmReplay();
    store = createReplayStore();
});

afterEach(() => {
    store.destroy();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

async function initStore(): Promise<void> {
    await store.initialiseRecording(ds.dataSource);
    store.setWasmReplay(mock.wasm, mock.module);
}

async function startPlayback(): Promise<void> {
    await initStore();
    store.play();
    ds.resolveFetch(makePdus(0, 15_000, 100));
    await vi.advanceTimersByTimeAsync(0);
}

async function seekTo(targetMs: number): Promise<void> {
    const done = store.seek(targetMs);
    // Resolve all pending fetches. The seek skips already-buffered chunks,
    // so the number of fetches depends on fetchedUntilMs.
    // advanceTimersByTimeAsync(1) also flushes yieldToEventLoop() setTimeout(0)s.
    while (store.playbackState.seeking) {
        while (ds.pendingCount > 0) ds.resolveFetch([]);
        await vi.advanceTimersByTimeAsync(1);
    }
    await done;
}

// =============================================================================
// 1. fetchAndPush — Buffer Management
// =============================================================================

describe('fetchAndPush', () => {
    test('advances fetchedUntilMs to toMs after fetch resolves', async () => {
        await initStore();
        store.play();
        ds.resolveFetch(makePdus(0, 12_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.fetchedUntilMs).toBe(15_000);
    });

    test('advances fetchedUntilMs even with empty fetch result', async () => {
        await initStore();
        store.play();
        ds.resolveFetch([]);
        await vi.advanceTimersByTimeAsync(0);

        expect(store.fetchedUntilMs).toBe(15_000);
    });

    test('calls pushPdu for each returned PDU', async () => {
        await initStore();
        store.play();
        const pdus = [makePdu(100), makePdu(200), makePdu(300)];
        ds.resolveFetch(pdus);
        await vi.advanceTimersByTimeAsync(0);

        expect(mock.wasm.pushPdu).toHaveBeenCalledTimes(3);
        expect(mock.wasm.pushPdu).toHaveBeenNthCalledWith(1, 100, 1, expect.any(Uint8Array));
        expect(mock.wasm.pushPdu).toHaveBeenNthCalledWith(2, 200, 1, expect.any(Uint8Array));
        expect(mock.wasm.pushPdu).toHaveBeenNthCalledWith(3, 300, 1, expect.any(Uint8Array));
    });

    test('does nothing after signal is aborted', async () => {
        await initStore();
        store.play();
        // Seek immediately aborts play's prefetch signal
        store.seek(30_000);
        // Resolve the now-aborted play fetch
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        // pushPdu should not have been called by the aborted fetch
        expect(mock.wasm.pushPdu).not.toHaveBeenCalled();
    });

    test('filters out-of-range PDUs', async () => {
        await initStore();
        store.play();
        // play() calls fetchAndPush(0, 15_000, signal).
        // Resolve with PDUs that include timestamps outside [0, 15_000):
        //   -50   → below fromMs, should be filtered
        //   100   → in range, should be pushed
        //   5_000 → in range, should be pushed
        //   15_000 → at toMs boundary (half-open), should be filtered
        const pdus = [makePdu(-50), makePdu(100), makePdu(5_000), makePdu(15_000)];
        ds.resolveFetch(pdus);
        await vi.advanceTimersByTimeAsync(0);

        // Only the 2 in-range PDUs should have been pushed
        expect(mock.wasm.pushPdu).toHaveBeenCalledTimes(2);
        expect(mock.wasm.pushPdu).toHaveBeenNthCalledWith(1, 100, 1, expect.any(Uint8Array));
        expect(mock.wasm.pushPdu).toHaveBeenNthCalledWith(2, 5_000, 1, expect.any(Uint8Array));
    });
});

// =============================================================================
// 2. play() — Start Playback
// =============================================================================

describe('play', () => {
    test('sets state, fetches, then starts rAF loop', async () => {
        await initStore();
        store.play();

        expect(store.playbackState.paused).toBe(false);
        expect(store.playbackState.waiting).toBe(true);

        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.playbackState.waiting).toBe(false);

        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).toHaveBeenCalled();
    });

    test('does not spawn second rAF loop when called twice', async () => {
        await startPlayback();

        store.play();
        ds.resolveFetch(makePdus(15_000, 30_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();

        expect(mock.wasm.renderTill).toHaveBeenCalledTimes(1);
    });

    test('respects pause during initial fetch', async () => {
        await initStore();
        store.play();

        expect(store.playbackState.waiting).toBe(true);
        store.pause();

        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.playbackState.paused).toBe(true);

        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('fetch failure sets error state', async () => {
        await initStore();
        store.play();

        ds.rejectFetch(new Error('network error'));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.loadState.status).toBe('error');
        expect(store.playbackState.paused).toBe(true);
        expect(store.playerError?.phase).toBe('playback');
    });
});

// =============================================================================
// 3. pause() — Stop Playback
// =============================================================================

describe('pause', () => {
    test('cancels rAF loop and sets paused', async () => {
        await startPlayback();

        store.pause();
        expect(store.playbackState.paused).toBe(true);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('while already paused is a no-op', async () => {
        await initStore();

        expect(store.playbackState.paused).toBe(true);
        store.pause();
        expect(store.playbackState.paused).toBe(true);
    });
});

// =============================================================================
// 4. seek() — Seeking
// =============================================================================

describe('seek', () => {
    test('forward seek fetches and renders in chunks', async () => {
        await startPlayback();
        await seekTo(5_000);
        // fetchedUntilMs = 15_000 after startPlayback + seekTo

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.mocked(ds.dataSource.fetch).mockClear();

        const seekDone = store.seek(20_000);
        expect(store.playbackState.seeking).toBe(true);

        // Buffered region [5_000, 15_000) rendered in one call to renderTill(15_000).
        // Unbuffered chunk [15_000, 20_000) fetched and rendered as renderTill(20_000).
        // Drain: resolve pending fetches and flush yieldToEventLoop() setTimeout(0)s.
        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }
        await seekDone;

        expect(ds.dataSource.fetch).toHaveBeenCalledTimes(1);
        expect(mock.wasm.renderTill).toHaveBeenCalledWith(15_000);
        expect(mock.wasm.renderTill).toHaveBeenCalledWith(20_000);
        expect(store.playbackState.seeking).toBe(false);
    });

    test('backward seek calls reset, fetches from 0', async () => {
        await startPlayback();
        await seekTo(20_000);

        vi.mocked(mock.wasm.reset).mockClear();
        vi.mocked(ds.dataSource.fetch).mockClear();

        store.seek(5_000);
        expect(mock.wasm.reset).toHaveBeenCalled();

        // 1 chunk: [0, 5000)
        ds.resolveFetch([]);
        await vi.advanceTimersByTimeAsync(0);

        expect(ds.dataSource.fetch).toHaveBeenCalledWith(0, 5_000, expect.anything());
    });

    test('clamps target to [0, duration]', async () => {
        await initStore();

        store.seek(-100);
        // Resolve any chunks (seek to 0 has no chunks since 0 < 0 is false)
        await vi.advanceTimersByTimeAsync(0);
        expect(store.elapsed).toBe(0);

        const seekDone = store.seek(999_999);
        // Resolve chunks for seek to 60000 (clamped)
        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }
        await seekDone;
        expect(store.elapsed).toBe(60_000);
    });

    test('suppresses canvas during seek, re-enables after', async () => {
        await startPlayback();

        const seekDone = store.seek(20_000);
        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }
        await seekDone;

        const calls = vi.mocked(mock.wasm.setUpdateCanvas).mock.calls;
        expect(calls[0]).toEqual([false]);
        expect(calls[calls.length - 1]).toEqual([true]);
        expect(mock.wasm.forceRedraw).toHaveBeenCalled();
    });

    test('rapid seek-seek-seek aborts previous seeks', async () => {
        await startPlayback();

        store.seek(10_000);
        store.seek(20_000);
        store.seek(30_000);

        // First two seeks' fetches were auto-rejected by abort signal.
        // Only the third seek's fetches are pending.
        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }

        expect(store.elapsed).toBe(30_000);
        expect(store.playbackState.seeking).toBe(false);
    });

    test('resumes rAF loop if not paused after seek', async () => {
        await startPlayback();

        const seekDone = store.seek(20_000);
        while (store.playbackState.seeking) {
            // Resolve with PDUs extending beyond target so there's buffer ahead
            while (ds.pendingCount > 0) ds.resolveFetch(makePdus(20_000, 35_000, 100));
            await vi.advanceTimersByTimeAsync(1);
        }
        await seekDone;

        expect(store.playbackState.paused).toBe(false);

        // After seek, bufferAhead may be 0 (fetchedUntilMs == elapsed).
        // The first tick enters critically-low and fires a refill fetch.
        // Resolve it so the loop resumes.
        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        if (ds.pendingCount > 0) {
            ds.resolveFetch(makePdus(20_000, 40_000, 100));
            for (let i = 0; i < 5; i++) await vi.advanceTimersByTimeAsync(1);
        }
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).toHaveBeenCalled();
    });

    test('does NOT resume loop if paused during seek', async () => {
        await startPlayback();

        const seekDone = store.seek(20_000);
        store.pause();

        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }
        await seekDone;

        expect(store.playbackState.paused).toBe(true);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('fetch error during seek sets error state', async () => {
        await startPlayback();
        // Seek beyond fetchedUntilMs (15_000) so a fetch actually fires
        store.seek(20_000);
        // The seek skips buffered chunks, then hits [15_000, 20_000) which needs a fetch.
        // Wait for the fetch to become pending, then reject it.
        while (ds.pendingCount === 0) {
            await vi.advanceTimersByTimeAsync(1);
        }
        ds.rejectFetch(new Error('seek network error'));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.loadState.status).toBe('error');
        expect(store.playbackState.paused).toBe(true);
        expect(store.playbackState.seeking).toBe(false);
        expect(store.playerError?.phase).toBe('seek');
    });

    test('forward seek skips fetch for already-buffered range', async () => {
        await startPlayback();
        // After startPlayback(): elapsed = 0, fetchedUntilMs = 15_000.
        // The rAF loop is scheduled but no frame has advanced yet.

        vi.mocked(ds.dataSource.fetch).mockClear();

        store.seek(25_000);
        // The seek renders the buffered region [0, 15_000) in one renderTill call,
        // then only fetches the unbuffered remainder [15_000, 25_000).

        while (ds.pendingCount > 0) {
            ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(0);
        }

        // Every fetch call's fromMs should be >= fetchedUntilMs (15_000).
        // No re-fetching of already-buffered data.
        const fetchCalls = vi.mocked(ds.dataSource.fetch).mock.calls;
        for (const [fromMs] of fetchCalls) {
            expect(fromMs).toBeGreaterThanOrEqual(15_000);
        }
    });
});

// =============================================================================
// 5. tick() — Render Loop
// =============================================================================

describe('tick', () => {
    test('advances elapsed by delta * speed', async () => {
        await startPlayback();
        store.setSpeed(2.0);

        const elapsedBefore = store.elapsed;
        vi.advanceTimersToNextFrame(); // 16ms
        const elapsedAfter = store.elapsed;

        expect(elapsedAfter - elapsedBefore).toBe(32); // 16ms * 2x speed
    });

    test('calls renderTill(elapsed) each frame', async () => {
        await startPlayback();

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();

        expect(mock.wasm.renderTill).toHaveBeenCalledTimes(1);
        expect(mock.wasm.renderTill).toHaveBeenCalledWith(store.elapsed);
    });

    test('pauses on session_ended', async () => {
        await startPlayback();
        mock.setSessionEnded();

        vi.advanceTimersToNextFrame();
        expect(store.playbackState.paused).toBe(true);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('pauses when elapsed >= duration', async () => {
        ds = createMockDataSource({ durationMs: 100 });
        mock = createMockWasmReplay();
        // Use a very low critically-low threshold so the buffer check doesn't
        // interfere — we want to test the end-of-recording pause, not buffer stalls.
        store = createReplayStore({ criticallyLowMs: 0 });

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        // Provide PDUs that span the full duration so fetchedUntilMs tracks correctly
        ds.resolveFetch(makePdus(0, 101, 10));
        await vi.advanceTimersByTimeAsync(0);

        // Advance frames until elapsed reaches duration (100ms / 16ms per frame ~ 7 frames)
        for (let i = 0; i < 20; i++) {
            vi.advanceTimersToNextFrame();
            if (store.playbackState.paused === true) break;
        }

        expect(store.elapsed).toBe(100);
        expect(store.playbackState.paused).toBe(true);
    });

    test('enters waiting state on critically low buffer', async () => {
        store = createReplayStore({ criticallyLowMs: 50_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        vi.advanceTimersToNextFrame();

        expect(store.playbackState.waiting).toBe(true);
        expect(ds.pendingCount).toBe(1);
    });

    test('resumes after critically-low fetch completes', async () => {
        // After play: fetchedUntilMs = 15_000, elapsed = 0, bufferAhead = 15_000.
        // Buffer health is checked before advancing elapsed, so bufferAhead = 15_000 - 0.
        // criticallyLowMs: 16_000 triggers a stall on the first tick (15_000 < 16_000).
        store = createReplayStore({ criticallyLowMs: 16_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        // Trigger critically-low (bufferAhead = 15_000 < 16_000)
        vi.advanceTimersToNextFrame();
        expect(store.playbackState.waiting).toBe(true);

        // Resolve the refill fetch. After this, fetchedUntilMs = 30_000,
        // bufferAhead = 30_000 - 0 = 30_000 > 16_000 — no longer critically low.
        while (store.playbackState.waiting) {
            if (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }

        expect(store.playbackState.waiting).toBe(false);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).toHaveBeenCalled();
    });

    test('pause during critically-low recovery prevents resume', async () => {
        store = createReplayStore({ criticallyLowMs: 50_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        // Trigger critically-low
        vi.advanceTimersToNextFrame();
        expect(store.playbackState.waiting).toBe(true);

        // Pause while waiting
        store.pause();

        // Resolve the refill fetch
        ds.resolveFetch(makePdus(15_000, 80_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.playbackState.paused).toBe(true);
        expect(store.playbackState.waiting).toBe(false);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('fires background prefetch on low buffer', async () => {
        store = createReplayStore({ lowThresholdMs: 50_000, criticallyLowMs: 500 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        vi.advanceTimersToNextFrame();

        // Background fetch started but loop still running
        expect(ds.pendingCount).toBe(1);
        expect(store.playbackState.waiting).toBe(false);
        expect(store.playbackState.paused).toBe(false);
    });

    test('consecutive ticks do not fire overlapping prefetches', async () => {
        // lowThresholdMs: 50_000 ensures buffer is always "low" (fetchedUntilMs = 15_000).
        // criticallyLowMs: 0 prevents the critically-low branch from firing.
        store = createReplayStore({ lowThresholdMs: 50_000, criticallyLowMs: 0 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        vi.mocked(ds.dataSource.fetch).mockClear();

        vi.advanceTimersToNextFrame(); // tick 1: fires background prefetch
        vi.advanceTimersToNextFrame(); // tick 2: should see in-flight prefetch and skip

        // Second tick should reuse the in-flight prefetch, not fire a new one.
        expect(ds.pendingCount).toBe(1);
    });

    test('does not advance elapsed when buffer is critically low', async () => {
        // criticallyLowMs: 50_000 ensures the buffer is immediately critically low
        // after the initial fetch (fetchedUntilMs = 15_000, bufferAhead = 15_000 < 50_000).
        store = createReplayStore({ criticallyLowMs: 50_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        const elapsedBefore = store.elapsed;
        vi.advanceTimersToNextFrame();

        // Buffer is critically low — tick should freeze without advancing elapsed.
        expect(store.elapsed).toBe(elapsedBefore);
    });

    test('renderTill error pauses and sets player error', async () => {
        await startPlayback();
        mock.setRenderError(new Error('wasm crash'));

        vi.advanceTimersToNextFrame();

        expect(store.playbackState.paused).toBe(true);
        expect(store.playerError?.message).toBe('wasm crash');
        expect(store.playerError?.phase).toBe('playback');
    });
});

// =============================================================================
// 6. initialiseRecording() — Load Lifecycle
// =============================================================================

describe('initialiseRecording', () => {
    test('successful open sets ready state with metadata', async () => {
        await store.initialiseRecording(ds.dataSource);

        expect(store.loadState.status).toBe('ready');
        expect(store.duration).toBe(60_000);
    });

    test('failed open sets error state', async () => {
        const failDs = createMockDataSource({ durationMs: 0, deferOpen: true });
        store.initialiseRecording(failDs.dataSource);
        failDs.rejectOpen(new Error('connection refused'));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.loadState.status).toBe('error');
        expect(store.playerError?.phase).toBe('init');
    });

    test('second initialiseRecording aborts the first', async () => {
        const ds1 = createMockDataSource({ durationMs: 30_000, deferOpen: true });
        const ds2 = createMockDataSource({ durationMs: 60_000 });

        store.initialiseRecording(ds1.dataSource);
        await store.initialiseRecording(ds2.dataSource);

        expect(ds1.dataSource.close).toHaveBeenCalledTimes(1);
        expect(store.loadState.status).toBe('ready');
        expect(store.duration).toBe(60_000);
    });
});

// =============================================================================
// 6b. initialiseRecording — Cleanup on re-init
// =============================================================================

describe('initialiseRecording cleanup', () => {
    test('during active playback: stops loop, closes old source, pauses', async () => {
        await startPlayback();

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(ds.dataSource.close).toHaveBeenCalledTimes(1);
        expect(store.playbackState.paused).toBe(true);
        expect(store.playbackState.waiting).toBe(false);
        expect(store.playbackState.seeking).toBe(false);
        expect(store.fetchedUntilMs).toBe(0);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('after failed open: still calls close, clears error', async () => {
        const failDs = createMockDataSource({ durationMs: 0, deferOpen: true });
        store.initialiseRecording(failDs.dataSource);
        failDs.rejectOpen(new Error('network error'));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.playerError?.phase).toBe('init');

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(failDs.dataSource.close).toHaveBeenCalledTimes(1);
        expect(store.playerError).toBeNull();
    });

    test('resets elapsed to 0', async () => {
        await startPlayback();
        await seekTo(5_000);
        expect(store.elapsed).toBe(5_000);

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(store.elapsed).toBe(0);
    });

    test('frees previous WASM instance', async () => {
        await initStore();

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(mock.wasm.free).toHaveBeenCalledTimes(1);
    });

    test('aborts in-flight seek', async () => {
        await startPlayback();

        store.seek(25_000);
        expect(store.playbackState.seeking).toBe(true);

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(store.playbackState.seeking).toBe(false);
    });

    test('during critically-low recovery: aborts recovery, closes old source', async () => {
        store = createReplayStore({ criticallyLowMs: 16_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        // Trigger critically-low (buffer 15_000 < threshold 16_000).
        vi.advanceTimersToNextFrame();
        expect(store.playbackState.waiting).toBe(true);

        const ds2 = createMockDataSource({ durationMs: 30_000 });
        await store.initialiseRecording(ds2.dataSource);

        expect(ds.dataSource.close).toHaveBeenCalledTimes(1);
        expect(store.playbackState.waiting).toBe(false);
        expect(store.duration).toBe(30_000);

        // The old IIFE is suspended on a pending mock fetch. Resolve it so the
        // IIFE can reach its signal.aborted guard and exit cleanly.
        while (ds.pendingCount > 0) {
            ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(0);
        }

        // The old IIFE must not have resumed playback on the new recording.
        expect(store.playbackState.paused).toBe(true);
    });
});

// =============================================================================
// 7. destroy() — Teardown
// =============================================================================

describe('destroy', () => {
    test('stops rAF loop during active playback', async () => {
        await startPlayback();

        vi.mocked(mock.wasm.renderTill).mockClear();
        store.destroy();

        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
        expect(store.playbackState.paused).toBe(true);
    });

    test('aborts active seek and restores canvas updates', async () => {
        await startPlayback();

        // Start a seek that will need fetches beyond the buffer
        store.seek(25_000);
        expect(store.playbackState.seeking).toBe(true);

        store.destroy();

        // Seek should be aborted — no more fetches pending
        expect(store.playbackState.seeking).toBe(false);
        expect(store.playbackState.paused).toBe(true);
    });

    test('calls dataSource.close()', async () => {
        await initStore();

        store.destroy();
        expect(ds.dataSource.close).toHaveBeenCalledTimes(1);
    });

    test('play and seek are no-ops after destroy', async () => {
        await startPlayback();
        store.destroy();

        // play() should be a no-op (dataSource and wasmReplay are null)
        store.play();
        expect(store.playbackState.paused).toBe(true);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();

        // seek() should be a no-op
        await store.seek(10_000);
        expect(store.elapsed).toBe(0);
    });

    test('clears ended state', async () => {
        await startPlayback();
        mock.setSessionEnded();
        vi.advanceTimersByTime(16);
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(true);

        store.destroy();
        expect(store.playbackState.ended).toBe(false);
    });

    test('during critically-low recovery does not resume playback', async () => {
        store = createReplayStore({ criticallyLowMs: 16_000 });
        ds = createMockDataSource({ durationMs: 60_000 });
        mock = createMockWasmReplay();

        await store.initialiseRecording(ds.dataSource);
        store.setWasmReplay(mock.wasm, mock.module);
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        // Trigger critically-low (bufferAhead = 15_000 < 16_000)
        vi.advanceTimersToNextFrame();
        expect(store.playbackState.waiting).toBe(true);

        // Destroy while the IIFE is awaiting the refill fetch
        store.destroy();

        // Resolve the now-aborted fetch — should not resume playback
        if (ds.pendingCount > 0) {
            ds.resolveFetch(makePdus(15_000, 60_000, 100));
            await vi.advanceTimersByTimeAsync(1);
        }

        expect(store.playbackState.paused).toBe(true);
        expect(store.playbackState.waiting).toBe(false);

        vi.mocked(mock.wasm.renderTill).mockClear();
        vi.advanceTimersToNextFrame();
        expect(mock.wasm.renderTill).not.toHaveBeenCalled();
    });

    test('frees WASM instance exactly once', async () => {
        await initStore();

        store.destroy();

        expect(mock.wasm.free).toHaveBeenCalledTimes(1);
    });

    test('double destroy does not double-free', async () => {
        await initStore();

        store.destroy();
        store.destroy();

        expect(mock.wasm.free).toHaveBeenCalledTimes(1);
    });
});

// =============================================================================
// 8. ended — End-of-Recording State
// =============================================================================

describe('ended', () => {
    test('ended is false by default', async () => {
        await initStore();
        expect(store.playbackState.ended).toBe(false);
    });

    test('tick sets ended when elapsed reaches duration', async () => {
        await startPlayback();
        // Drive elapsed to duration by advancing many frames.
        // The store's tick loop fires background prefetches when buffer runs low,
        // so we must drain pending fetches each iteration to avoid stalling.
        store.setSpeed(3);
        for (let i = 0; i < 5000; i++) {
            if (store.playbackState.ended) break;
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            vi.advanceTimersByTime(16);
            await vi.advanceTimersByTimeAsync(0);
        }
        expect(store.playbackState.ended).toBe(true);
        expect(store.playbackState.paused).toBe(true);
    });

    test('tick sets ended on session_ended from WASM', async () => {
        await startPlayback();
        mock.setSessionEnded();
        vi.advanceTimersByTime(16);
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(true);
        expect(store.playbackState.paused).toBe(true);
    });

    test('play() clears ended', async () => {
        await startPlayback();
        mock.setSessionEnded();
        vi.advanceTimersByTime(16);
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(true);

        mock.resetRenderBehavior();
        store.play();
        ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(false);
    });

    test('seek() clears ended', async () => {
        await startPlayback();
        mock.setSessionEnded();
        vi.advanceTimersByTime(16);
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(true);

        mock.resetRenderBehavior();
        await seekTo(0);
        expect(store.playbackState.ended).toBe(false);
    });

    test('togglePlayback() resets and plays when ended', async () => {
        await startPlayback();
        mock.setSessionEnded();
        vi.advanceTimersByTime(16);
        await vi.advanceTimersByTimeAsync(0);
        expect(store.playbackState.ended).toBe(true);

        mock.resetRenderBehavior();
        store.togglePlayback();
        // togglePlayback calls reset().then(() => play())
        // First: seek(0) completes
        while (store.playbackState.seeking) {
            while (ds.pendingCount > 0) ds.resolveFetch([]);
            await vi.advanceTimersByTimeAsync(1);
        }
        // Then: play() fires and needs its initial fetch resolved
        while (ds.pendingCount > 0) ds.resolveFetch(makePdus(0, 15_000, 100));
        await vi.advanceTimersByTimeAsync(0);

        expect(store.playbackState.ended).toBe(false);
        expect(store.playbackState.paused).toBe(false);
        expect(store.elapsed).toBe(0);
    });
});

// =============================================================================
// 9. setWasmReplay — WASM Instance Lifecycle
// =============================================================================

describe('setWasmReplay', () => {
    test('frees WASM instance when init() throws', async () => {
        await store.initialiseRecording(ds.dataSource);

        const failingWasm = createMockWasmReplay();
        vi.mocked(failingWasm.wasm.init).mockImplementation(() => {
            throw new Error('init failed');
        });

        store.setWasmReplay(failingWasm.wasm, failingWasm.module);

        expect(failingWasm.wasm.free).toHaveBeenCalledTimes(1);
        expect(store.loadState.status).toBe('error');
    });
});
