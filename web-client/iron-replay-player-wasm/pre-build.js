import { spawn } from 'child_process';

const run = async (command, cwd) => {
    const buildCommand = spawn(command, { stdio: 'pipe', shell: true, cwd });

    buildCommand.stdout.on('data', (data) => console.log(`${data}`));
    buildCommand.stderr.on('data', (data) => console.error(`${data}`));

    return new Promise((resolve, reject) => {
        buildCommand.on('close', (code) => {
            if (code !== 0) {
                reject(new Error(`Process exited with non-zero code: ${code}`));
            } else {
                resolve(code);
            }
        });
    });
};

await run('cargo xtask web build-replay', '../../');
