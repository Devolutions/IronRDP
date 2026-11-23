import { cp, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { argv, execPath } from 'node:process';

let noWasm = false;

const assetReplayPlayerFolder = './static/iron-replay-player';
const assetReplayPlayerWasmFolder = './static/iron-replay-player-wasm';

argv.forEach((val, index) => {
    if (index === 2 && val === 'no-wasm') {
        noWasm = true;
    }
});

const wasmBinarySource = '../../crates/ironrdp-web-replay/pkg/ironrdp_web_replay_bg.wasm';

const copyDistFiles = async () => {
    console.log('Copying dist files…');
    await rm(assetReplayPlayerFolder, { recursive: true, force: true });
    await rm(assetReplayPlayerWasmFolder, { recursive: true, force: true });

    await cp('../iron-replay-player/dist', assetReplayPlayerFolder, { recursive: true });
    await cp('../iron-replay-player-wasm/dist', assetReplayPlayerWasmFolder, { recursive: true });

    // The .wasm binary is referenced via ?url import in the JS bundle but is not
    // inlined — it must be served alongside the JS from the same directory.
    await cp(wasmBinarySource, `${assetReplayPlayerWasmFolder}/ironrdp_web_replay_bg.wasm`);
    console.log('Dist files copied successfully');
};

// Step 1: Build WASM crate (unless skipped)
if (!noWasm) {
    execFileSync('cargo', ['xtask', 'web', 'build-replay'], { cwd: '../../', stdio: 'inherit' });
} else {
    // Build the libraries when WASM has already been built or is pre-existing.
    execFileSync(execPath, ['./node_modules/vite/bin/vite.js', 'build'], {
        cwd: '../iron-replay-player-wasm',
        stdio: 'inherit',
    });
    execFileSync(execPath, ['./node_modules/vite/bin/vite.js', 'build'], {
        cwd: '../iron-replay-player',
        stdio: 'inherit',
    });

    await copyDistFiles();
}
