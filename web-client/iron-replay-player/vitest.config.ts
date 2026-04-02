import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
    plugins: [svelte({ hot: false })],
    test: {
        include: ['tests/**/*.test.ts'],
        exclude: ['tests/browser/**'],
        environment: 'jsdom',
    },
    resolve: {
        conditions: ['browser'],
    },
});
