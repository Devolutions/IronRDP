import type { ReplayDataSource, ReplayMetadata, PlayerError } from '../interfaces/ReplayDataSource.js';
import type { LoadState } from '../interfaces/LoadState.js';
import type { PlaybackState } from '../interfaces/PlaybackState.js';
import type { ReplayConfigInstance, ReplayModule, WasmReplayInstance } from '../interfaces/ReplayModule.js';

export interface BufferConfig {
    targetMs: number;
    lowThresholdMs: number;
    criticallyLowMs: number;
    seekChunkMs: number;
}

export const DEFAULT_BUFFER_CONFIG: BufferConfig = {
    targetMs: 15_000,
    lowThresholdMs: 5_000,
    criticallyLowMs: 500,
    seekChunkMs: 5_000,
};

export function createReplayStore(bufferOverrides?: Partial<BufferConfig>) {
    const bufferConfig = { ...DEFAULT_BUFFER_CONFIG, ...bufferOverrides };

    // --- Load state ---
    let loadState = $state<LoadState>({ status: 'idle' });
    let playerError = $state<PlayerError | null>(null);

    // --- Data source state ---
    let dataSource: ReplayDataSource | null = null;
    let duration = $state(0);
    let totalPdus = $state(0);
    let recordingMetadata: ReplayMetadata | null = null;

    /*
     * BUFFER MODEL
     *
     * - `elapsed`: current playback position. Advances each rAF tick by
     *   delta × speed. Render loop calls renderTill(elapsed) to decode PDUs
     *   up to this point.
     *
     * - `fetchedUntilMs`: high-water mark — the furthest point (in ms) for
     *   which we have requested data from the data source. Advanced
     *   monotonically by fetchAndPush (Math.max(fetchedUntilMs, toMs)).
     *   Only reset to 0 on backward seek.
     *
     * - `bufferAhead = fetchedUntilMs - elapsed`: how much data is available
     *   ahead of the playhead. The tick loop compares this against thresholds:
     *     - criticallyLowMs: freeze playhead, show "Buffering...", fetch more
     *     - lowThresholdMs: fire background prefetch (no stall)
     *
     * All callers of fetchAndPush use fetchedUntilMs as fromMs (except the
     * seek loop, which advances its own cursor through chunk boundaries).
     * This means fetches are contiguous and non-overlapping: each call picks
     * up exactly where the previous one left off.
     */
    let playbackState = $state<PlaybackState>({ paused: true, waiting: false, seeking: false, ended: false });
    let elapsed = $state(0);
    let speed = $state(1.0);
    let fetchedUntilMs = $state(0);

    // --- Internal refs ---
    let rafId: number | null = null;
    let lastTimestamp: DOMHighResTimeStamp | null = null;
    let wasmReplay: WasmReplayInstance | null = $state(null);

    // --- Abort controllers ---
    let openAbort = new AbortController();
    let seekAbort = new AbortController();
    let prefetchAbort = new AbortController();

    /*
     * Tracks the in-flight background data fetch promise. Used by:
     * - The low-threshold branch in tick() to prevent overlapping fetches.
     * - The critically-low branch in tick() to await an in-flight fetch
     *   instead of aborting it and wasting network resources.
     * Reset to null in seek() and destroy() after aborting the prefetch controller.
     */
    let prefetchPromise: Promise<void> | null = null;

    /** Cancel the previous operation and return a fresh controller. */
    function resetAbort(controller: AbortController): AbortController {
        controller.abort();
        return new AbortController();
    }

    /** Cancel in-flight seek and background prefetch operations. */
    function cancelInflightWork(): void {
        seekAbort = resetAbort(seekAbort);
        prefetchAbort = resetAbort(prefetchAbort);
        prefetchPromise = null;
    }

    function yieldToEventLoop(): Promise<void> {
        return new Promise((resolve) => setTimeout(resolve, 0));
    }

    /*
     * RENDER LOOP MANAGEMENT
     * The rAF loop must be running if and only if:
     *   !paused && !seeking && !waiting
     * These helpers enforce this invariant. All playback state changes
     * MUST go through updatePlaybackState() to keep the loop in sync.
     */

    /** Cancel the rAF loop unconditionally. */
    function stopLoop(): void {
        if (rafId !== null) {
            cancelAnimationFrame(rafId);
            rafId = null;
        }
    }

    /**
     * Start the rAF loop if all conditions are met.
     * Resets lastTimestamp to prevent a time-jump on the first tick
     * after a pause, seek, or buffer stall.
     */
    function tryStartLoop(): void {
        if (!playbackState.paused && !playbackState.seeking && !playbackState.waiting) {
            if (rafId === null) {
                lastTimestamp = performance.now();
                rafId = requestAnimationFrame(tick);
            }
        }
    }

    /**
     * Centralized state mutator. Automatically stops or starts the rAF
     * loop based on the resulting state. All code that changes playback
     * state MUST use this function — never spread playbackState directly.
     */
    function updatePlaybackState(updates: Partial<PlaybackState>): void {
        playbackState = { ...playbackState, ...updates };

        if (playbackState.paused || playbackState.seeking || playbackState.waiting) {
            stopLoop();
        } else {
            tryStartLoop();
        }
    }

    // --- Player error helpers ---
    // First-error-wins: once an error is set, subsequent errors are logged
    // and discarded until the consumer calls clearError().
    function setPlayerError(phase: PlayerError['phase'], error: unknown): void {
        if (playerError !== null) {
            console.warn(`[replay] dropping ${phase} error (first-error-wins):`, error);
            return;
        }
        playerError = {
            message: error instanceof Error ? error.message : String(error),
            phase,
            cause: error,
        };
    }

    function clearError(): void {
        if (playerError !== null) {
            loadState = playerError.phase === 'init' ? { status: 'idle' } : { status: 'ready' };
        }
        playerError = null;
    }

    /*
     * Fetch PDUs from the data source and push them into the WASM buffer.
     *
     * Invariants:
     * - Rejects inverted/empty ranges (fromMs >= toMs).
     * - Filters out-of-range PDUs to keep the WASM buffer lean.
     * - Advances fetchedUntilMs monotonically (only seek() backward resets it).
     */
    async function fetchAndPush(fromMs: number, toMs: number, signal: AbortSignal): Promise<void> {
        if (!dataSource || !wasmReplay) return;

        if (fromMs >= toMs) {
            console.warn(`[replay] fetchAndPush: inverted range [${fromMs}, ${toMs}), skipping`);
            return;
        }

        const pdus = await dataSource.fetch(fromMs, toMs, signal);

        if (signal.aborted) return;

        // Push PDUs into WASM buffer for playback
        for (const pdu of pdus) {
            if (pdu.timestampMs < fromMs || pdu.timestampMs >= toMs) continue;
            wasmReplay.pushPdu(pdu.timestampMs, pdu.source, pdu.data);
        }

        fetchedUntilMs = Math.max(fetchedUntilMs, toMs);
    }

    // --- Load recording metadata ---
    async function initialiseRecording(source: ReplayDataSource): Promise<void> {
        // --- Previous recording teardown ---
        // Abort in-flight work before replacing the data source — prevents
        // tick() from calling fetch() on the new source before open() resolves.
        cancelInflightWork();
        updatePlaybackState({ paused: true, waiting: false, seeking: false, ended: false });

        wasmReplay?.free();
        wasmReplay = null;

        if (dataSource) {
            try {
                dataSource.close();
            } catch (e) {
                console.warn('[replay] close() threw during re-init:', e);
            }
        }

        playerError = null; // clearError() would reset loadState

        // --- New recording setup ---
        openAbort = resetAbort(openAbort);
        const { signal } = openAbort;

        dataSource = source;
        loadState = { status: 'loading' };
        duration = 0;
        totalPdus = 0;
        elapsed = 0;
        fetchedUntilMs = 0;
        recordingMetadata = null;

        try {
            const metadata = await dataSource.open(signal);
            if (signal.aborted) return;

            duration = metadata.durationMs;
            totalPdus = metadata.totalPdus;
            recordingMetadata = metadata;
            loadState = { status: 'ready' };
        } catch (e: unknown) {
            if (signal.aborted) return; // superseded by a newer initialiseRecording call
            loadState = {
                status: 'error',
                message: e instanceof Error ? e.message : 'failed to open data source',
            };
            setPlayerError('init', e);
        }
    }

    // --- Set load error (called by component on WASM constructor failure) ---
    function setLoadError(message: string): void {
        loadState = { status: 'error', message };
        setPlayerError('init', new Error(message));
    }

    // --- Wire in WASM replay instance (called from component after WASM loads) ---
    function setWasmReplay(replay: WasmReplayInstance, replayModule: ReplayModule): void {
        wasmReplay?.free();
        wasmReplay = replay;

        // Build config from recording metadata if the module provides ReplayConfig.
        let config: ReplayConfigInstance | undefined;
        if (replayModule.ReplayConfig != null && recordingMetadata != null) {
            config = new replayModule.ReplayConfig();
            if (recordingMetadata.ioChannelId != null) {
                config.io_channel_id = recordingMetadata.ioChannelId;
            }
            if (recordingMetadata.userChannelId != null) {
                config.user_channel_id = recordingMetadata.userChannelId;
            }
            if (recordingMetadata.shareId != null) {
                config.share_id = recordingMetadata.shareId;
            }
        }

        try {
            wasmReplay.init(config);
        } catch (err: unknown) {
            wasmReplay.free();
            wasmReplay = null;
            setLoadError(err instanceof Error ? err.message : 'replay init failed');
            return;
        }
    }

    // --- Seek ---
    async function seek(targetMs: number): Promise<void> {
        if (!dataSource || !wasmReplay) return;

        targetMs = Math.max(0, Math.min(targetMs, duration));

        cancelInflightWork();
        const { signal } = seekAbort;

        updatePlaybackState({ seeking: true, waiting: true, ended: false });

        // Direction decision: compare targetMs against current elapsed
        let processFrom = elapsed;
        const isBackwardSeek = targetMs < elapsed;

        // Immediately show head at target position — avoids visible snap-back
        elapsed = targetMs;

        try {
            if (isBackwardSeek) {
                // Backward seek — reset WASM, restart from 0
                // DataSource is stateless — no reset needed.
                wasmReplay.reset();
                processFrom = 0;
                fetchedUntilMs = 0;
            }

            // Suppress canvas updates during intermediate chunks
            wasmReplay.setUpdateCanvas(false);

            /*
             * CHUNKED FAST-FORWARD
             *
             * For forward seeks, prefetch may have already loaded data ahead.
             * Render the buffered region in one call, then fetch+render only
             * the unbuffered remainder in chunks.
             *
             * For backward seeks, fetchedUntilMs was reset to 0 above,
             * so the buffered-region render is skipped and every chunk is fetched.
             */

            // Fast-forward WASM through already-buffered region (no fetch needed)
            if (processFrom < fetchedUntilMs) {
                const renderTo = Math.min(fetchedUntilMs, targetMs);
                wasmReplay.renderTill(renderTo);
            }

            // Fetch + render only the unbuffered remainder in seekChunkMs-sized
            // chunks. Each chunk: fetch PDUs from the data source, push them
            // into WASM, then fast-forward WASM's decoder to that point.
            // We yield between chunks so the UI thread can process a new seek
            // if the user drags the scrubber (abort checks after every await).
            let current = Math.max(processFrom, fetchedUntilMs);
            while (current < targetMs) {
                if (signal.aborted) return; // new seek arrived before this chunk

                const chunkEnd = Math.min(current + bufferConfig.seekChunkMs, targetMs);

                await fetchAndPush(current, chunkEnd, signal);
                if (signal.aborted) return; // new seek arrived during fetch

                wasmReplay.renderTill(chunkEnd);
                current = chunkEnd;

                await yieldToEventLoop(); // let UI process (e.g. another seek)
                if (signal.aborted) return;
            }

            // Final render with canvas updates re-enabled.
            // All PDUs up to targetMs were processed by the chunk loop above.
            // Re-enable canvas updates and force a single blit of the final frame.
            wasmReplay.setUpdateCanvas(true);
            wasmReplay.forceRedraw();
        } catch (e) {
            if (signal.aborted) return; // superseded — discard error silently
            loadState = {
                status: 'error',
                message: e instanceof Error ? e.message : 'seek failed',
            };
            updatePlaybackState({ seeking: false, waiting: false, paused: true });
            setPlayerError('seek', e);
            return;
        } finally {
            // Re-enable canvas updates unless this seek was superseded.
            if (!signal.aborted) {
                try {
                    wasmReplay?.setUpdateCanvas(true);
                } catch {
                    // WASM instance freed or in error state — already reported above.
                }
            }
        }

        if (signal.aborted) return;

        updatePlaybackState({ seeking: false, waiting: false });
    }

    // --- Play: record intent, fill buffer, then start rAF loop ---
    async function play(): Promise<void> {
        if (!dataSource || !wasmReplay) return;

        updatePlaybackState({ paused: false, waiting: true, ended: false });

        const signal = prefetchAbort.signal;
        try {
            await fetchAndPush(fetchedUntilMs, fetchedUntilMs + bufferConfig.targetMs, signal);
        } catch (e) {
            if (signal.aborted) return; // superseded by seek or destroy
            loadState = {
                status: 'error',
                message: e instanceof Error ? e.message : 'failed to fetch PDUs',
            };
            updatePlaybackState({ paused: true, waiting: false });
            setPlayerError('playback', e);
            return;
        }

        updatePlaybackState({ waiting: false });
    }

    // --- Pause: record intent, cancel rAF loop (fetch continues unaffected) ---
    function pause(): void {
        updatePlaybackState({ paused: true });
    }

    // --- Guard: can the player accept playback commands? ---
    function canControlPlayback(): boolean {
        return wasmReplay !== null && dataSource !== null && !playbackState.seeking;
    }

    // --- Toggle: single entry point for play/pause from canvas click ---
    function togglePlayback(): void {
        if (!canControlPlayback()) return;
        if (playbackState.ended) {
            // Safe from re-entry: seek(0) synchronously sets seeking=true
            // before its first await, and canControlPlayback() rejects
            // calls while seeking — so a rapid second toggle is a no-op.
            reset()
                .then(() => play())
                .catch(() => {
                    // Errors already reported via setPlayerError() inside seek()/play().
                });
        } else if (playbackState.paused) {
            play();
        } else {
            pause();
        }
    }

    // --- Reset: seek to beginning, preserving play/pause state ---
    function reset(): Promise<void> {
        return seek(0);
    }

    // --- Speed: set playback speed ---
    function setSpeed(value: number): void {
        if (!Number.isFinite(value) || value <= 0) return;
        speed = value;
    }

    /* rAF TICK CALLBACK */
    function tick(now: DOMHighResTimeStamp): void {
        if (!dataSource || !wasmReplay || lastTimestamp === null) return;

        /* ── 1. BUFFER HEALTH CHECK (before advancing time) ────────────── */
        const bufferAhead = fetchedUntilMs - elapsed;

        if (bufferAhead <= bufferConfig.criticallyLowMs) {
            /*
             * Buffer is critically low or empty. Freeze immediately:
             * - Do NOT advance elapsed (prevents playhead drift).
             * - Show "Buffering..." overlay.
             * - If a background prefetch is already in-flight, await it
             *   instead of aborting it (saves network resources).
             * - If we still need data after that, fetch synchronously.
             * - Resume the loop only when we have enough data.
             */
            updatePlaybackState({ waiting: true });

            const signal = prefetchAbort.signal;
            (async () => {
                /* If a background prefetch is already in-flight, wait for it
                 * to complete rather than aborting it and wasting the network
                 * request. The prefetch may have nearly finished. */
                if (prefetchPromise !== null) {
                    try {
                        await prefetchPromise;
                    } catch {
                        /* Error handled by the prefetch's own catch block */
                    }
                }

                /* A seek may have arrived while we were awaiting the prefetch.
                 * If so, the seek owns the state machine now — bail out. */
                if (signal.aborted) return;

                /* Re-check: the awaited prefetch may have filled the buffer. */
                if (fetchedUntilMs - elapsed <= bufferConfig.criticallyLowMs) {
                    /* Still need data — fetch directly and await.
                     * No prefetchPromise tracking needed here: the loop is
                     * stopped (waiting: true), so no ticks can read it.
                     * seek() nulls prefetchPromise explicitly if it interrupts. */
                    try {
                        await fetchAndPush(fetchedUntilMs, fetchedUntilMs + bufferConfig.targetMs, signal);
                    } catch (e) {
                        if (signal.aborted) return;
                        setPlayerError('playback', e);
                        updatePlaybackState({ waiting: false, paused: true });
                        return;
                    }
                }

                if (signal.aborted) return;
                updatePlaybackState({ waiting: false });
            })().catch((e) => {
                // Safety net for unexpected errors not caught by inner try/catch blocks.
                // Known fetchAndPush errors are handled inline and never reach here.
                // signal.aborted means a seek/destroy took ownership of the state machine.
                if (signal.aborted) return;
                console.error('[replay] unexpected error in critically-low recovery:', e);
                setPlayerError('playback', e);
                updatePlaybackState({ waiting: false, paused: true });
            });
            return;
        }

        /* ── 2. ADVANCE TIME ───────────────────────────────────────────── */
        const delta = now - lastTimestamp;
        lastTimestamp = now;
        elapsed = Math.min(elapsed + delta * speed, duration);

        /* ── 3. RENDER ─────────────────────────────────────────────────── */
        let renderResult;
        try {
            renderResult = wasmReplay.renderTill(elapsed);
        } catch (e) {
            setPlayerError('playback', e);
            updatePlaybackState({ paused: true });
            return;
        }

        /* ── 4. END CONDITIONS ─────────────────────────────────────────── */
        if (renderResult.session_ended || elapsed >= duration) {
            updatePlaybackState({ paused: true, ended: true });
            return;
        }

        /* ── 5. BACKGROUND PREFETCH (low threshold) ────────────────────── */
        /*
         * Recompute bufferAhead after advancing elapsed — the value from
         * step 1 is stale by one frame's delta. At high playback speeds
         * (10x+) this difference can be significant (~160ms).
         */
        const currentBufferAhead = fetchedUntilMs - elapsed;
        if (currentBufferAhead < bufferConfig.lowThresholdMs && prefetchPromise === null) {
            // Identity check: only clear prefetchPromise if it's still this fetch.
            const inflightFetch = fetchAndPush(
                fetchedUntilMs,
                fetchedUntilMs + bufferConfig.targetMs,
                prefetchAbort.signal,
            )
                .catch((e) => {
                    if (!prefetchAbort.signal.aborted) setPlayerError('playback', e);
                })
                .finally(() => {
                    if (prefetchPromise === inflightFetch) prefetchPromise = null;
                });
            prefetchPromise = inflightFetch;
        }

        /* ── 6. SCHEDULE NEXT TICK ─────────────────────────────────────── */
        rafId = requestAnimationFrame(tick);
    }

    function destroy(): void {
        openAbort = resetAbort(openAbort);
        cancelInflightWork();
        updatePlaybackState({ paused: true, waiting: false, seeking: false, ended: false });
        wasmReplay?.free();
        wasmReplay = null;
        recordingMetadata = null;
        if (dataSource) {
            try {
                dataSource.close();
            } catch {
                /* fire-and-forget */
            }
        }
        dataSource = null;
    }

    return {
        // Load state
        get loadState() {
            return loadState;
        },
        get playerError() {
            return playerError;
        },
        get duration() {
            return duration;
        },
        get totalPdus() {
            return totalPdus;
        },
        initialiseRecording,
        setLoadError,
        clearError,

        // Playback state
        get playbackState() {
            return playbackState;
        },
        get elapsed() {
            return elapsed;
        },
        get speed() {
            return speed;
        },
        get fetchedUntilMs() {
            return fetchedUntilMs;
        },

        // Playback controls
        canControlPlayback,
        play,
        pause,
        seek,
        reset,
        togglePlayback,
        setSpeed,
        setWasmReplay,
        destroy,
    };
}
