import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { playwright } from '@vitest/browser-playwright';

export default defineConfig({
    plugins: [
        svelte({
            hot: false,
            compilerOptions: {
                // Disable custom element compilation so vitest-browser-svelte's
                // render() receives a standard Svelte component, not a CE class.
                customElement: false,
            },
        }),
    ],
    test: {
        include: ['tests/browser/**/*.browser.test.ts'],
        browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: 'chromium' }],
        },
    },
    resolve: {
        conditions: ['browser'],
    },
});
