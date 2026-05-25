import { defineConfig } from 'vite';
import topLevelAwait from 'vite-plugin-top-level-await';
import dtsPlugin from 'vite-plugin-dts';

// https://vitejs.dev/config/
export default defineConfig({
    build: {
        lib: {
            entry: './src/main.ts',
            name: 'IronReplayPlayerWasm',
            fileName: 'IronReplayPlayerWasm',
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
