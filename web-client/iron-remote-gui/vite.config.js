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
            name: 'IronRemoteGui',
            formats: ['umd', 'es'],
        },
    },
    server: {
        fs: {
            strict: false,
        },
    },
    rollupOptions: {
        external: ['rxjs'],
    },
    plugins: [
        svelte(),
        wasm(),
        topLevelAwait(),
        dtsPlugin({
            rollupTypes: true,
        }),
    ],
});
//# sourceMappingURL=vite.config.js.map
