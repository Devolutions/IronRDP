import wasm_init, {
    Replay,
    ReplayConfig,
    PduSource,
} from '../../../crates/ironrdp-web-replay/pkg/ironrdp_web_replay.js';

export { PduSource, ReplayConfig };

/**
 * Initialize the WASM module. Must be called once before constructing Replay instances.
 */
export async function init(): Promise<void> {
    await wasm_init();
}

/**
 * ReplayBackend satisfies the ReplayModule interface expected by iron-replay-player.
 * Pass this as the `module` prop to <iron-replay-player>.
 */
export const ReplayBackend = {
    Replay,
    ReplayConfig,
    PduSource,
};
