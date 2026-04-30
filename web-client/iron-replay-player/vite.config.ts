import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import topLevelAwait from 'vite-plugin-top-level-await';
// https://vitejs.dev/config/
// Note: vite-plugin-wasm is intentionally absent — WASM loading is handled entirely
// by iron-replay-player-wasm, not by this UI package.
//
// vite-plugin-dts is intentionally omitted — its tsc pass cannot handle Svelte 5
// runes ($state, $derived, etc.) in .svelte.ts files. The JS bundle builds fine
// without it, and consumers use the <iron-replay-player> custom element via HTML,
// not TS imports.
export default defineConfig({
    build: {
        lib: {
            entry: './src/main.ts',
            name: 'IronReplayPlayer',
            fileName: 'IronReplayPlayer',
            formats: ['es'],
        },
    },
    server: {
        fs: {
            strict: false,
        },
    },
    plugins: [svelte(), topLevelAwait()],
});
