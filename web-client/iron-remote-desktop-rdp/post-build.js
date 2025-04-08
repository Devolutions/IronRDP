import fs from 'fs-extra';

const sourceWasmFile = '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm';
const assetWasmFile = './dist/ironrdp_web_bg.wasm';

const copyWasmFile = async () => {
    try {
        await fs.remove(assetWasmFile);
        await fs.copy(sourceWasmFile, assetWasmFile);
        console.log('Wasm file was copied successfully');
    } catch (err) {
        console.error(`An error occurred while copying wasm file: ${err}`);
    }
};

const renameWasmFile = async (path, new_path) => {
    try {
        await fs.rename(path, new_path);
    } catch (err) {
        console.error(`Rename failed: ${err}`);
    }
};

await renameWasmFile(
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg1.wasm',
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm',
);
await copyWasmFile();
