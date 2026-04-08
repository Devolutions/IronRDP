/// <reference types="vitest" />
import { defineConfig } from 'vite';
import topLevelAwait from 'vite-plugin-top-level-await';
import dtsPlugin from 'vite-plugin-dts';

// https://vitejs.dev/config/
export default defineConfig({
    test: {
        environment: 'jsdom',
    },
    build: {
        lib: {
            entry: './src/main.ts',
            name: 'IronRemoteDesktopRdp',
            formats: ['es'],
        },
    },
    server: {
        fs: {
            strict: false,
        },
    },
    plugins: [
        topLevelAwait(),
        dtsPlugin({
            rollupTypes: true,
        }),
    ],
});
