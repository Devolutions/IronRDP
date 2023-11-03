import { spawn } from 'child_process';

let run = async function (command, cwd) {
    return new Promise((resolve) => {
        const buildCommand = spawn(command, { stdio: 'pipe', shell: true, cwd: cwd });

        buildCommand.stdout.on('data', (data) => {
            console.log(`${data}`);
        });

        buildCommand.stderr.on('data', (data) => {
            console.error(`${data}`);
        });

        buildCommand.on('close', (code) => {
            console.log(`child process exited with code ${code}`);
            resolve();
        });
    });
};

await run('wasm-pack build --target web', '../../crates/ironrdp-web');
