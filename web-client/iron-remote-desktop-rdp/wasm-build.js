import { spawn } from 'child_process';

const run = async (command, cwd) => {
    try {
        const buildCommand = spawn(command, {
            stdio: 'pipe',
            shell: true,
            cwd: cwd,
            env: { ...process.env, RUSTFLAGS: '-Ctarget-feature=+simd128,+bulk-memory' },
        });

        buildCommand.stdout.on('data', (data) => {
            console.log(`${data}`);
        });

        buildCommand.stderr.on('data', (data) => {
            console.error(`${data}`);
        });

        const exitCode = await new Promise((resolve, reject) => {
            buildCommand.on('close', (code) => {
                if (code !== 0) {
                    reject(new Error(`Process exited with non-zero code: ${code}`));
                }
                resolve(code);
            });
        });

        console.log(`Child process exited with code: ${exitCode}`);
    } catch (err) {
        console.error(`Process run failed: ${err}`);
    }
};

await run('wasm-pack build --target web', '../../crates/ironrdp-web');
