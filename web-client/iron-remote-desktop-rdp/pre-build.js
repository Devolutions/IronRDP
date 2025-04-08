import fs from 'fs-extra';

const renameWasmFile = async (path, new_path) => {
    try {
        await fs.rename(path, new_path);
    } catch (err) {
        console.error(`Rename failed: ${err}`);
    }
};

// Renaming the file is temporary solution to prevent vite from inlining the wasm asset.
// In post-build.js file is renamed back.
// Issue reference: https://github.com/vitejs/vite/issues/4454
await renameWasmFile(
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm',
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg1.wasm',
);
