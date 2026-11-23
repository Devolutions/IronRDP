import { init, ReplayBackend } from '../../static/iron-replay-player-wasm/IronReplayPlayerWasm.js';
import type { ReplayDataSource } from './ReplayDataSource.types.js';

export interface IronReplayPlayerElement extends HTMLElement {
    module: unknown;
    dataSource: unknown;
}

/**
 * Initialize the WASM backend and wire a data source to a player element.
 *
 * Waits one animation frame for the DOM to settle (the player element is
 * conditionally rendered), then sets properties to start initialization.
 *
 * Returns `null` on success or an error message string on failure.
 * Callers do not need try/catch -- all errors are captured internally.
 */
export async function initPlayer(
    getPlayerEl: () => IronReplayPlayerElement | null,
    dataSource: ReplayDataSource,
): Promise<string | null> {
    try {
        await init();

        // Wait one frame for the conditionally-rendered element to mount.
        await new Promise((r) => requestAnimationFrame(r));

        const playerEl = getPlayerEl();
        if (!playerEl) {
            return 'player element not found after mount';
        }

        playerEl.module = ReplayBackend;
        playerEl.dataSource = dataSource;

        return null;
    } catch (err) {
        return err instanceof Error ? err.message : String(err);
    }
}
