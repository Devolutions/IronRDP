import { render } from 'vitest-browser-svelte';
import { vi } from 'vitest';
import IronReplayPlayer from '../../src/iron-replay-player.svelte';
import { createMockDataSource } from '../helpers/mock-data-source.js';
import { createMockWasmReplay } from '../helpers/mock-wasm-replay.js';
import type { PlayerApi } from '../../src/interfaces/PlayerApi.js';

export interface MountOptions {
    /** Mock recording duration in milliseconds. Default: 30_000. */
    durationMs?: number;
}

/**
 * Mount the player and get it to the `ready` state with a captured PlayerApi.
 *
 * Sets up mock data source (deferred open) and mock WASM, registers a `ready`
 * event listener on `document` before rendering (the event bubbles from an
 * internal div during a $effect chain), then resolves open() to trigger the
 * full initialization sequence.
 */
export async function mountPlayer(options?: MountOptions) {
    const durationMs = options?.durationMs ?? 30_000;

    const mockDataSource = createMockDataSource({ durationMs, deferOpen: true });
    const mockWasm = createMockWasmReplay();

    // Configure the Replay constructor to return our mock wasm instance.
    // Must use `function` (not arrow) so it can be called with `new`.
    (mockWasm.module.Replay as ReturnType<typeof vi.fn>).mockImplementation(function () {
        return mockWasm.wasm;
    });

    // Register the ready listener BEFORE render — the event fires during a
    // $effect chain and we must not miss it.
    const readyPromise = new Promise<PlayerApi>((resolve) => {
        document.addEventListener(
            'ready',
            (e) => {
                resolve((e as CustomEvent<{ playerApi: PlayerApi }>).detail.playerApi);
            },
            { once: true },
        );
    });

    const screen = render(IronReplayPlayer, {
        props: {
            dataSource: mockDataSource.dataSource,
            module: mockWasm.module,
        },
    });

    // Resolve the deferred open() → triggers $effect chain → WASM constructor → ready event.
    mockDataSource.resolveOpen();

    const api = await readyPromise;

    return { api, screen, mockDataSource, mockWasm };
}

/**
 * Mount the player but leave it in the `loading` state (open() not resolved).
 *
 * Useful for testing loading UI, error injection before ready, etc.
 */
export async function mountPlayerPartial(options?: MountOptions) {
    const durationMs = options?.durationMs ?? 30_000;

    const mockDataSource = createMockDataSource({ durationMs, deferOpen: true });
    const mockWasm = createMockWasmReplay();

    // Configure the Replay constructor (same as mountPlayer, for consistency
    // if a test later resolves open manually).
    (mockWasm.module.Replay as ReturnType<typeof vi.fn>).mockImplementation(function () {
        return mockWasm.wasm;
    });

    const screen = render(IronReplayPlayer, {
        props: {
            dataSource: mockDataSource.dataSource,
            module: mockWasm.module,
        },
    });

    return { screen, mockDataSource, mockWasm };
}
