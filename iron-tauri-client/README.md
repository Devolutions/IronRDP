# Tauri GUI Client

## How to build

> Gui client is still in early stage development and everything can change (mostly the framework use on web side)

### Prerequisites

- Node.js (npm): https://nodejs.org/en/
- wasm-pack: https://rustwasm.github.io/wasm-pack/installer/
- rust... : https://www.rust-lang.org/tools/install

### Steps

#### Dev

- run `wasm-pack build` in `./ffi/wasm` folder
- run `npm install` in `./iron-svelte-client` folder
- run `npm run build-tauri` in `./iron-svelte-client` folder
- run `npm install` in `./iron-tauri-client` folder
- finally, run `npm run tauri dev` in `./iron-tauri-client` folder

#### Build executable

To build the executable, run the step detailed in Dev section then, instead the last one, 
- run `npm run tauri build`. 
- You can execute the application directly from `./iron-tauri-client/src-tauri/target/release` or you can install it on your computer by executing the msi file from `./iron-tauri-client/src-tauri/target/release/bundle/msi`