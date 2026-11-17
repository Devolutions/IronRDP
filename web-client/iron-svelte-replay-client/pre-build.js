import { cp, rm } from 'node:fs/promises';
import { spawn } from 'child_process';
import { argv } from 'node:process';

let noWasm = false;

const assetReplayPlayerFolder = './static/iron-replay-player';
const assetReplayPlayerWasmFolder = './static/iron-replay-player-wasm';

argv.forEach((val, index) => {
    if (index === 2 && val === 'no-wasm') {
        noWasm = true;
    }
});

const run = async (command, cwd) => {
    const buildCommand = spawn(command, { stdio: 'pipe', shell: true, cwd });

    buildCommand.stdout.on('data', (data) => console.log(`${data}`));
    buildCommand.stderr.on('data', (data) => console.error(`${data}`));

    return new Promise((resolve, reject) => {
        buildCommand.on('close', (code) => {
            if (code !== 0) {
                reject(new Error(`Process exited with code ${code}`));
            } else {
                resolve();
            }
        });
    });
};

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
    await run('cargo xtask web build-replay', '../../');
}

// Step 2: Build iron-replay-player-wasm lib (build-alone: WASM already built above or pre-existing)
await run('npm run build-alone', '../iron-replay-player-wasm');

// Step 3: Build iron-replay-player lib
await run('npm run build', '../iron-replay-player');

// Step 4: Copy dist folders into static/
await copyDistFiles();
