<svelte:options
    customElement={{
        tag: 'iron-replay-player',
        shadow: 'none',
    }}
/>

<script lang="ts">
    import { untrack } from 'svelte';
    import { createReplayStore } from './services/replay-store.svelte.js';
    import type { ReplayModule, WasmReplayInstance } from './interfaces/ReplayModule.js';
    import type { PlayerApi } from './interfaces/PlayerApi.js';
    import type { ReplayDataSource } from './interfaces/ReplayDataSource.js';
    import SeekBar from './ui/SeekBar.svelte';
    import PlaybackControls from './ui/PlaybackControls.svelte';

    let {
        dataSource,
        module,
    }: {
        dataSource: ReplayDataSource;
        module: ReplayModule;
    } = $props();

    const store = createReplayStore();
    let canvas: HTMLCanvasElement;
    let playerDiv: HTMLDivElement;
    let wasmReady = $state(false);
    let isFullscreen = $state(false);
    let controlsVisible = $state(true);
    let controlsTimeout: ReturnType<typeof setTimeout> | null = null;

    $effect(() => {
        return () => {
            if (controlsTimeout) clearTimeout(controlsTimeout);
            if (seekDebounce) clearTimeout(seekDebounce);
            if (actionOverlayTimeout) clearTimeout(actionOverlayTimeout);
            if (seekOverlayTimeout) clearTimeout(seekOverlayTimeout);
            // Cancel RAF loop and abort any in-flight seek before freeing WASM memory.
            // This ensures the seek's finally block sees signal.aborted = true and does
            // not call setUpdateCanvas() on the freed instance.
            store.destroy();
        };
    });

    function showControls(): void {
        controlsVisible = true;
        if (controlsTimeout) clearTimeout(controlsTimeout);
        if (!store.playbackState.paused) {
            controlsTimeout = setTimeout(() => {
                controlsVisible = false;
            }, 3000);
        }
    }

    $effect(() => {
        // Track only the `dataSource` prop, NOT the store state that
        // initialiseRecording reads/writes internally (playbackState,
        // loadState, wasmReplay, etc).  Without untrack the effect would
        // re-trigger itself on every store mutation, causing
        // effect_update_depth_exceeded.
        const ds = dataSource;
        if (ds) untrack(() => store.initialiseRecording(ds));
    });

    // Reset wasmReady when a new load starts so WASM re-initializes for the new recording.
    $effect(() => {
        if (store.loadState.status === 'loading') {
            wasmReady = false;
        }
    });

    $effect(() => {
        if (store.loadState.status !== 'ready') return;
        if (!canvas || wasmReady) return;

        // new module.Replay(canvas) calls a Rust constructor returning Result<Replay, JsValue>.
        // wasm-bindgen converts Err into a JS throw, so wrap in try/catch.
        let replay: WasmReplayInstance;
        try {
            replay = new module.Replay(canvas);
        } catch (err: unknown) {
            console.error('Failed to construct WASM Replay engine:', err);
            untrack(() => store.setLoadError(err instanceof Error ? err.message : 'WASM init failed'));
            return;
        }

        // untrack: setWasmReplay reads/writes wasmReplay ($state) internally;
        // tracking it here would cause a spurious re-run (the wasmReady guard
        // prevents it from looping, but the extra run is unnecessary).
        untrack(() => store.setWasmReplay(replay, module));
        wasmReady = true;

        const playerApi: PlayerApi = {
            load: (newDataSource: ReplayDataSource) => store.initialiseRecording(newDataSource),
            play: () => store.play(),
            pause: () => store.pause(),
            togglePlayback: () => store.togglePlayback(),
            seek: (positionMs: number) => store.seek(positionMs),
            setSpeed: (s: number) => store.setSpeed(s),
            getSpeed: () => store.speed,
            getElapsedMs: () => store.elapsed,
            getDurationMs: () => store.duration,
            isPaused: () => store.playbackState.paused,
            getLoadState: () => store.loadState,
            getPlayerError: () => store.playerError,
            clearError: () => store.clearError(),
            reset: () => store.reset(),
        };

        playerDiv.dispatchEvent(
            new CustomEvent('ready', {
                detail: { playerApi },
                bubbles: true,
                composed: true,
            }),
        );
    });

    $effect(() => {
        if (store.playerError !== null) {
            playerDiv?.dispatchEvent(
                new CustomEvent('error', {
                    detail: store.playerError,
                    bubbles: true,
                    composed: true,
                }),
            );
        }
    });

    $effect(() => {
        const handler = () => {
            isFullscreen = document.fullscreenElement === playerDiv;
        };
        document.addEventListener('fullscreenchange', handler);
        return () => document.removeEventListener('fullscreenchange', handler);
    });

    $effect(() => {
        if (store.playbackState.paused) {
            if (controlsTimeout) {
                clearTimeout(controlsTimeout);
                controlsTimeout = null;
            }
            controlsVisible = true;
        } else {
            showControls();
        }
    });

    function toggleFullscreen(): void {
        if (!document.fullscreenElement) {
            playerDiv.requestFullscreen();
        } else {
            document.exitFullscreen();
        }
    }

    const isBuffering = $derived(store.playbackState.waiting);
    const isEnded = $derived(store.playbackState.ended);
    const canPlay = $derived(store.canControlPlayback());

    type ActionOverlayKind = 'play' | 'pause' | null;
    let actionOverlay = $state<ActionOverlayKind>(null);
    let actionOverlayTimeout: ReturnType<typeof setTimeout> | null = null;

    function showActionOverlay(kind: 'play' | 'pause'): void {
        actionOverlay = kind;
        if (actionOverlayTimeout) clearTimeout(actionOverlayTimeout);
        actionOverlayTimeout = setTimeout(() => {
            actionOverlay = null;
        }, 600);
    }

    type SeekOverlayKind = 'forward' | 'backward' | null;
    let seekOverlay = $state<SeekOverlayKind>(null);
    let seekOverlayTimeout: ReturnType<typeof setTimeout> | null = null;

    function showSeekOverlay(direction: 'forward' | 'backward'): void {
        seekOverlay = direction;
        if (seekOverlayTimeout) clearTimeout(seekOverlayTimeout);
        seekOverlayTimeout = setTimeout(() => {
            seekOverlay = null;
        }, 600);
    }

    const SEEK_STEP_MS = 5_000;
    let seekDebounce: ReturnType<typeof setTimeout> | null = null;
    let pendingSeekTarget: number | null = null;

    function handleCanvasClick(): void {
        if (!canPlay) return;
        const wasEnded = store.playbackState.ended;
        const wasPaused = store.playbackState.paused;
        store.togglePlayback();
        if (wasEnded) {
            showActionOverlay('play');
        } else {
            showActionOverlay(wasPaused ? 'play' : 'pause');
        }
    }

    function handleReplay(e: MouseEvent): void {
        e.stopPropagation();
        store.reset()
            .then(() => store.play())
            .catch((err) => console.error('[replay] restart failed:', err));
    }

    function handlePlayerKeydown(e: KeyboardEvent): void {
        if (!canPlay) return;

        switch (e.key) {
            case ' ':
            case 'Enter':
            case 'k': {
                e.preventDefault();
                const wasEnded = store.playbackState.ended;
                const wasPaused = store.playbackState.paused;
                store.togglePlayback();
                if (wasEnded) {
                    showActionOverlay('play');
                } else {
                    showActionOverlay(wasPaused ? 'play' : 'pause');
                }
                break;
            }
            case 'ArrowRight':
            case 'l':
                e.preventDefault();
                handleSeekKey(SEEK_STEP_MS);
                break;
            case 'ArrowLeft':
            case 'j':
                e.preventDefault();
                handleSeekKey(-SEEK_STEP_MS);
                break;
            case 'Home':
                e.preventDefault();
                store.seek(0);
                break;
            case 'End':
                e.preventDefault();
                store.seek(store.duration);
                break;
            default:
                return;
        }
    }

    function handleSeekKey(deltaMs: number): void {
        const base = pendingSeekTarget ?? store.elapsed;
        const target = Math.max(0, Math.min(base + deltaMs, store.duration));
        pendingSeekTarget = target;
        showSeekOverlay(deltaMs > 0 ? 'forward' : 'backward');

        if (seekDebounce !== null) clearTimeout(seekDebounce);
        seekDebounce = setTimeout(() => {
            if (pendingSeekTarget !== null) {
                store.seek(pendingSeekTarget);
                pendingSeekTarget = null;
            }
            seekDebounce = null;
        }, 150);
    }
</script>

<div
    class="__irp-replay-player"
    bind:this={playerDiv}
    tabindex={canPlay ? 0 : -1}
    onkeydown={handlePlayerKeydown}
>
    {#if store.loadState.status === 'loading' || (store.loadState.status === 'ready' && !wasmReady)}
        <p class="__irp-loading-text">Loading recording...</p>
    {:else if store.loadState.status === 'error'}
        <p class="__irp-error">Error: {store.loadState.message}</p>
    {/if}

    <div
        class="__irp-canvas-container"
        onmousemove={showControls}
        onclick={handleCanvasClick}
    >
        {#if isBuffering}
            <div class="__irp-buffering-overlay">
                <span class="__irp-buffering-label">Buffering...</span>
            </div>
        {/if}
        {#if actionOverlay}
            <div class="__irp-action-overlay">
                <div class="__irp-action-pill">
                    {actionOverlay === 'play' ? '\u25B6' : '\u23F8'}
                </div>
            </div>
        {/if}
        {#if seekOverlay}
            <div class="__irp-action-overlay">
                <div class="__irp-seek-pill">
                    <span>{seekOverlay === 'forward' ? '\u25B6\u25B6' : '\u25C0\u25C0'}</span>
                    <span>5s</span>
                </div>
            </div>
        {/if}
        <!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role -->
        <canvas bind:this={canvas} role="img" aria-label="RDP session replay"></canvas>

        {#if isEnded}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="__irp-ended-overlay" onclick={handleReplay}>
                <button class="__irp-restart-btn" aria-label="Replay from beginning">
                    <span class="__irp-restart-icon">{'\u21BB'}</span>
                </button>
                <span class="__irp-restart-label">Replay</span>
            </div>
        {/if}

        {#if store.loadState.status === 'ready' && wasmReady}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events (propagation barrier -- not an interactive element) -->
            <div class="__irp-controls-overlay" class:__irp-hidden={!controlsVisible} onclick={(e) => e.stopPropagation()}>
                <SeekBar
                    elapsed={store.elapsed}
                    duration={store.duration}
                    fetchedUntilMs={store.fetchedUntilMs}
                    waiting={store.playbackState.waiting}
                    onseekend={(ms) => store.seek(ms)}
                    onseekkey={(deltaMs) => handleSeekKey(deltaMs)}
                    seekStepMs={SEEK_STEP_MS}
                />
                <PlaybackControls
                    paused={store.playbackState.paused}
                    waiting={store.playbackState.waiting}
                    canPlay={canPlay}
                    elapsed={store.elapsed}
                    duration={store.duration}
                    speed={store.speed}
                    isFullscreen={isFullscreen}
                    onplay={() => store.play()}
                    onpause={() => store.pause()}
                    onreset={() => store.reset()}
                    onspeedchange={(s) => store.setSpeed(s)}
                    onfullscreen={toggleFullscreen}
                />
            </div>
        {/if}
    </div>
</div>

<style>
    /* All styles are :global because:
       1. Sub-component classes (SeekBar, PlaybackControls) live in separate Svelte files
          and would be stripped by the scoping compiler if not marked global.
       2. shadow: 'none' means styles inject into document <head>, not a shadow root,
          so global rules reach child elements correctly. */

    :global(.__irp-replay-player) {
        background: #000;
        position: relative;
        font-family: system-ui, -apple-system, sans-serif;
        width: 100%;
        height: 100%;
        display: flex;
        flex-direction: column;
    }

    :global(.__irp-replay-player:focus) {
        outline: none;
    }

    :global(.__irp-replay-player:focus-visible) {
        outline: 2px solid rgba(74, 158, 255, 0.5);
        outline-offset: -2px;
    }

    :global(.__irp-loading-text) {
        color: rgba(255, 255, 255, 0.5);
        padding: 12px 16px;
        margin: 0;
        font-size: 14px;
        font-family: monospace;
    }

    :global(.__irp-error) {
        color: #f87171;
        padding: 12px 16px;
        margin: 0;
        font-size: 14px;
    }

    :global(.__irp-canvas-container) {
        position: relative;
        flex: 1;
        min-height: 0;
        width: 100%;
        background: #000;
        display: flex;
        align-items: center;
        justify-content: center;
        overflow: hidden;
    }

    :global(.__irp-canvas-container canvas) {
        display: block;
        max-width: 100%;
        max-height: 100%;
        width: auto;
        height: auto;
    }

    :global {
        @keyframes __irp-fade-out {
            0% { opacity: 0.9; }
            70% { opacity: 0.9; }
            100% { opacity: 0; }
        }
    }

    :global(.__irp-action-overlay) {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        pointer-events: none;
        z-index: 4;
        animation: __irp-fade-out 0.6s ease-out forwards;
    }

    :global(.__irp-action-pill) {
        background: rgba(0, 0, 0, 0.65);
        border-radius: 50%;
        width: 56px;
        height: 56px;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #fff;
        font-size: 22px;
        backdrop-filter: blur(4px);
    }

    :global(.__irp-ended-overlay) {
        position: absolute;
        inset: 0;
        background: rgba(0, 0, 0, 0.6);
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 12px;
        z-index: 5;
        cursor: pointer;
    }

    :global(.__irp-restart-btn) {
        background: rgba(255, 255, 255, 0.12);
        border: 2px solid rgba(255, 255, 255, 0.3);
        border-radius: 50%;
        width: 64px;
        height: 64px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background 0.15s ease, border-color 0.15s ease;
    }

    :global(.__irp-restart-btn:hover) {
        background: rgba(255, 255, 255, 0.2);
        border-color: rgba(255, 255, 255, 0.5);
    }

    :global(.__irp-restart-icon) {
        color: #fff;
        font-size: 24px;
    }

    :global(.__irp-restart-label) {
        color: rgba(255, 255, 255, 0.7);
        font-size: 14px;
        font-weight: 500;
    }

    :global(.__irp-seek-pill) {
        background: rgba(0, 0, 0, 0.65);
        border-radius: 24px;
        padding: 10px 20px;
        display: flex;
        align-items: center;
        gap: 6px;
        color: #fff;
        font-size: 15px;
        font-weight: 500;
        backdrop-filter: blur(4px);
    }

    :global(.__irp-buffering-overlay) {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(0, 0, 0, 0.5);
        z-index: 3;
    }

    :global(.__irp-buffering-label) {
        color: #fff;
        font-size: 18px;
        font-weight: 500;
    }

    :global(.__irp-controls-overlay) {
        position: absolute;
        bottom: 0;
        left: 0;
        right: 0;
        background: linear-gradient(to top, rgba(0, 0, 0, 0.88) 0%, transparent 100%);
        padding: 32px 16px 12px;
        z-index: 2;
        transition: opacity 0.3s ease;
    }

    :global(.__irp-controls-overlay.__irp-hidden) {
        opacity: 0;
        pointer-events: none;
        visibility: hidden;
        transition: opacity 0.3s ease, visibility 0s linear 0.3s;
    }

    /* Seekbar */
    :global(.__irp-seekbar) {
        width: 100%;
        padding: 16px 0;
        cursor: default;
        box-sizing: border-box;
    }

    :global(.__irp-seekbar-track) {
        position: relative;
        width: 100%;
        height: 4px;
        background: rgba(255, 255, 255, 0.15);
        border-radius: 2px;
        overflow: visible;
        transition: height 0.15s ease;
    }

    :global(.__irp-seekbar.__irp-interactive) {
        cursor: pointer;
    }

    :global(.__irp-seekbar:focus) {
        outline: none;
    }

    :global(.__irp-seekbar:focus-visible .__irp-seekbar-head) {
        box-shadow: 0 0 0 3px rgba(74, 158, 255, 0.5);
        width: 16px;
        height: 16px;
    }

    :global(.__irp-seekbar.__irp-interactive:hover .__irp-seekbar-track) {
        height: 6px;
    }

    :global(.__irp-seekbar-buffer) {
        position: absolute;
        left: 0;
        top: 0;
        height: 100%;
        background: rgba(255, 255, 255, 0.3);
        border-radius: 2px;
        pointer-events: none;
    }

    :global(.__irp-seekbar-progress) {
        position: absolute;
        left: 0;
        top: 0;
        height: 100%;
        background: #4a9eff;
        border-radius: 2px;
        pointer-events: none;
    }

    :global(.__irp-seekbar-head) {
        position: absolute;
        top: 50%;
        width: 12px;
        height: 12px;
        background: #4a9eff;
        border-radius: 50%;
        transform: translate(-50%, -50%);
        pointer-events: none;
        transition: opacity 0.15s ease, width 0.15s ease, height 0.15s ease;
        box-shadow: 0 0 4px rgba(74, 158, 255, 0.6);
    }

    :global(.__irp-seekbar.__irp-interactive:hover .__irp-seekbar-head) {
        width: 16px;
        height: 16px;
    }

    :global(.__irp-seekbar-head.__irp-waiting) {
        opacity: 0.5;
    }

    /* PlaybackControls */
    :global(.__irp-controls-bar) {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 0;
    }

    :global(.__irp-controls-left) {
        display: flex;
        align-items: center;
        gap: 10px;
    }

    :global(.__irp-controls-right) {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    :global(.__irp-play-btn) {
        width: 32px;
        height: 32px;
        border-radius: 50%;
        background: transparent;
        color: #fff;
        border: none;
        font-size: 13px;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
        transition: background 0.15s ease;
    }

    :global(.__irp-play-btn:hover:not(:disabled)) {
        background: rgba(255, 255, 255, 0.15);
    }

    :global(.__irp-play-btn:disabled) {
        opacity: 0.4;
        cursor: not-allowed;
    }

    :global(.__irp-time-display) {
        font-size: 13px;
        color: #ccc;
        white-space: nowrap;
        font-variant-numeric: tabular-nums;
        font-family: monospace;
    }

    :global(.__irp-speed-selector) {
        position: relative;
    }

    :global(.__irp-speed-btn) {
        font-size: 12px;
        color: #ccc;
        background: transparent;
        border: 1px solid rgba(255, 255, 255, 0.25);
        border-radius: 4px;
        padding: 3px 8px;
        cursor: pointer;
        white-space: nowrap;
        transition: border-color 0.15s ease, color 0.15s ease;
    }

    :global(.__irp-speed-btn:hover) {
        border-color: rgba(255, 255, 255, 0.5);
        color: #fff;
    }

    :global(.__irp-speed-popup) {
        position: absolute;
        bottom: calc(100% + 4px);
        right: 0;
        background: #1c1c1c;
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 8px;
        box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
        overflow: hidden;
        z-index: 10;
        min-width: 180px;
    }

    :global(.__irp-speed-popup-heading) {
        padding: 12px 16px 10px;
        font-size: 14px;
        font-weight: 500;
        color: #fff;
        border-bottom: 1px solid rgba(255, 255, 255, 0.1);
        white-space: nowrap;
    }

    :global(.__irp-speed-popup-item) {
        display: flex;
        align-items: center;
        width: 100%;
        padding: 10px 16px;
        font-size: 14px;
        color: #ccc;
        background: transparent;
        border: none;
        cursor: pointer;
        white-space: nowrap;
        text-align: left;
        gap: 10px;
    }

    :global(.__irp-speed-popup-item:hover) {
        background: rgba(255, 255, 255, 0.06);
    }

    :global(.__irp-speed-popup-item.__irp-active) {
        color: #fff;
    }

    :global(.__irp-speed-popup-check) {
        width: 14px;
        font-size: 13px;
        color: #fff;
        flex-shrink: 0;
    }

    :global(.__irp-fullscreen-btn) {
        font-size: 16px;
        background: transparent;
        border: none;
        color: #ccc;
        cursor: pointer;
        padding: 4px 6px;
        border-radius: 4px;
        line-height: 1;
        transition: color 0.15s ease;
    }

    :global(.__irp-fullscreen-btn:hover) {
        color: #fff;
    }

    :global(.__irp-replay-player:fullscreen),
    :global(.__irp-replay-player:-webkit-full-screen) {
        width: 100vw;
        height: 100vh;
        display: flex;
        flex-direction: column;
    }

    :global(.__irp-replay-player:fullscreen .__irp-canvas-container),
    :global(.__irp-replay-player:-webkit-full-screen .__irp-canvas-container) {
        flex: 1;
        height: 100%;
    }

    :global(.__irp-replay-player:fullscreen canvas),
    :global(.__irp-replay-player:-webkit-full-screen canvas) {
        max-width: 100%;
        max-height: 100%;
        width: auto;
        height: auto;
    }
</style>
