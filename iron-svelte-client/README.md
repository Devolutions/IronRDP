# SvelteKit UI for IronRDP.

The ui is used both for Tauri Desktop App and Browser App.

## Tauri

Please [read the Readme](../iron-tauri-client/) from `iron-tauri-client`

## Web Client

> WebClient is build with [SvelteKit](https://kit.svelte.dev/). 
> It's a simple wrapper around Iron-Remote-Gui to demo the usage of the API.
> The core of the WebClient is in iron-remote-gui folder who's built as web-component.

### Requirement
You need to run npm install in [iron-remote-gui](../iron-remote-gui/) before going further.
### Run in dev
- Run `npm install`
- Run dev server with all require build by `npm run dev-all`
- Build dist files by `npm run build`

Files are builded in `./iron-svelte-client/build/browser`

You can start the dev server with three different command: 
- `dev` - Run only the application. 
- `dev-all` - Build wasm and iron-remote-gui before starting. 
- `dev-no-wasm` - Build only iron-remote-gui before starting.