import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';
import dtsPlugin from 'vite-plugin-dts';

// https://vitejs.dev/config/
export default defineConfig({
    build: {
        lib: {
            entry: './src/main.ts',
            name: 'IronRemoteDesktop',
            formats: ['es'],
        },
    },
    // Component tests mount Svelte components, which needs Svelte's browser build.
    resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
    server: {
        fs: {
            strict: false,
        },
    },
    plugins: [
        // Under vitest (which runs its own Vite 5), vite-plugin-svelte 5 can't preprocess styles; they are plain CSS.
        svelte(process.env.VITEST ? { preprocess: [] } : {}),
        wasm(),
        topLevelAwait(),
        dtsPlugin({
            rollupTypes: true,
        }),
    ],
    test: {
        globals: true,
        environment: 'jsdom',
        setupFiles: './src/test/setup.ts',
    },
});
