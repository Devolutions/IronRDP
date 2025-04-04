import * as fs from 'fs-extra';
import { spawn } from 'child_process';
import { argv } from 'node:process';

let noWasm = false;

let assetIronRemoteDesktopFolder = './static/iron-remote-desktop';
let assetIronRemoteDesktopRdpFolder = './static/iron-remote-desktop-rdp';

argv.forEach((val, index) => {
    if (index === 2 && val === 'no-wasm') {
        noWasm = true;
    }
});

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

let copyCoreFiles = async function () {
    console.log('Copying core files…');
    await fs.remove(assetIronRemoteDesktopFolder);
    await fs.remove(assetIronRemoteDesktopRdpFolder);
    return new Promise((resolve) => {
        let source = '../iron-remote-desktop/dist';
        let destination = assetIronRemoteDesktopFolder;
        let sourceRdp = '../iron-remote-desktop-rdp/dist';
        let destinationRdp = assetIronRemoteDesktopRdpFolder;

        fs.copy(source, destination, function (err) {
            if (err) {
                console.log('An error occurred while copying core files.');
                return console.error(err);
            }
            console.log('Core files were copied successfully');
            fs.copy(sourceRdp, destinationRdp, function (err) {
                if (err) {
                    console.log('An error occurred while copying core files.');
                    return console.error(err);
                }
                console.log('Core files were copied successfully');
            });
            resolve();
        });

    });
};

let buildCommand = 'npm run build';
if (noWasm) {
    buildCommand = 'npm run build-alone';
}

await run(buildCommand, '../iron-remote-desktop');
await run(buildCommand, '../iron-remote-desktop-rdp');
await copyCoreFiles();