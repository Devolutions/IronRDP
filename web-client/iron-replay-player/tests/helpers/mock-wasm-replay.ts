import { vi } from 'vitest';
import type {
    ReplayModule,
    WasmReplayInstance,
    RenderResult,
    ReplayConfigInstance,
} from '../../src/interfaces/ReplayModule.js';

class MockReplayConfig implements ReplayConfigInstance {
    io_channel_id?: number;
    user_channel_id?: number;
    share_id?: number;
}

type RenderBehavior = { type: 'default' } | { type: 'session_ended' } | { type: 'error'; error: Error };

export function createMockWasmReplay() {
    let renderBehavior: RenderBehavior = { type: 'default' };

    function makeRenderResult(targetMs: number): RenderResult {
        switch (renderBehavior.type) {
            case 'session_ended':
                return {
                    current_time_ms: targetMs,
                    pdus_processed: 0,
                    resolution_changed: false,
                    session_ended: true,
                };
            case 'error':
                throw renderBehavior.error;
            default:
                return {
                    current_time_ms: targetMs,
                    pdus_processed: 0,
                    resolution_changed: false,
                    session_ended: false,
                };
        }
    }

    const wasm: WasmReplayInstance = {
        free: vi.fn(),
        init: vi.fn(),
        pushPdu: vi.fn(),
        renderTill: vi.fn((targetMs: number) => makeRenderResult(targetMs)),
        reset: vi.fn(),
        setUpdateCanvas: vi.fn(),
        forceRedraw: vi.fn(),
    };

    const module: ReplayModule = {
        Replay: vi.fn() as unknown as { new (canvas: HTMLCanvasElement): WasmReplayInstance },
        PduSource: { Client: 0, Server: 1 },
        ReplayConfig: MockReplayConfig,
    };

    return {
        wasm,
        module,

        /** Make renderTill return session_ended: true (sticky). */
        setSessionEnded(): void {
            renderBehavior = { type: 'session_ended' };
        },

        /** Make renderTill throw the given error (sticky). */
        setRenderError(error: Error): void {
            renderBehavior = { type: 'error', error };
        },

        /** Restore renderTill to default behavior. */
        resetRenderBehavior(): void {
            renderBehavior = { type: 'default' };
        },
    };
}
