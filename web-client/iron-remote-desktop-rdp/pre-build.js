import fs from 'fs-extra';

let renameWasmFile = async function (path, new_path) {
    return new Promise((resolve) => {
        fs.rename(path, new_path, function (err) {
            if (err) {
                console.error(`${err}`);
            }
        });
        resolve();
    });
};

// Renaming the file is temporary solution to prevent vite from inlining the wasm asset.
// In post-build.js file is renamed back.
// Issue reference: https://github.com/vitejs/vite/issues/4454
await renameWasmFile(
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg.wasm',
    '../../crates/ironrdp-web/pkg/ironrdp_web_bg1.wasm',
);
