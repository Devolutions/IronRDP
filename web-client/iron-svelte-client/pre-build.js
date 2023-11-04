﻿import * as fs from 'fs-extra';
import {spawn} from 'child_process';
import * as path from 'path';
import {fileURLToPath} from 'url';
import { argv } from 'node:process';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

let noWasm = false;

let assetIronRemoteGuiFolder = './static/iron-remote-gui';

argv.forEach((val, index) => {
    if (index === 2 && val === 'no-wasm') {
        noWasm = true;
    }
});

let run = async function (command, cwd) {
    return new Promise(resolve => {
        const buildCommand = spawn(command, {stdio: "pipe", shell: true, cwd: cwd});

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
    await fs.remove(assetIronRemoteGuiFolder);
    return new Promise(resolve => {
        let source = '../iron-remote-gui/dist';
        let destination = assetIronRemoteGuiFolder;

        fs.copy(source, destination, function (err) {
            if (err) {
                console.log('An error occurred while copying core files.');
                return console.error(err);
            }
            console.log('Core files were copied successfully');
            resolve();
        });
    })
}

let buildCommand = 'npm run build';
if (noWasm) {
    buildCommand = 'npm run build-alone';
}

await run(buildCommand, '../iron-remote-gui');
await copyCoreFiles();
