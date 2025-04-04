import fs from 'fs-extra';

let sourceWasmFile = '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm';
let assetWasmFile = './dist/ironrdp_web_bg.wasm';

let copyWasmFile = async function () {
    await fs.remove(assetWasmFile);
    return new Promise((resolve) => {
        fs.copy(sourceWasmFile, assetWasmFile, function (err) {
            if (err) {
                console.log('An error occurred while copying wasm file');
                return console.error(err);
            }
            console.log('Wasm file was copied successfully');
            resolve();
        });

    });
};

let renameWasmFile = async function (path, new_path) {
    return new Promise((resolve) => {
        fs.rename(path, new_path, function (err) {
            if (err) {
                console.error(`${err}`);
            }
        });
        resolve();
    });
}

await renameWasmFile('../../crates/ironrdp-web/pkg/ironrdp_web_bg1.wasm', '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm');
await copyWasmFile();
