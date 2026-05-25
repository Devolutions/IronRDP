import { describe, it, expect } from 'vitest';
import { page } from 'vitest/browser';
import { mountPlayer, mountPlayerPartial } from './setup.js';
import { makePdus } from '../helpers/mock-data-source.js';

describe('overlay visibility', () => {
    it('loading text shows during initialization', async () => {
        await mountPlayerPartial();

        const loading = page.getByText('Loading recording...');
        await expect.element(loading).toBeVisible();
    });

    it('loading text disappears after init completes', async () => {
        const { mockDataSource } = await mountPlayerPartial();

        const loading = page.getByText('Loading recording...');
        await expect.element(loading).toBeVisible();

        mockDataSource.resolveOpen();

        await expect.element(loading).not.toBeVisible();
    });

    it('buffering overlay appears when data source delays', async () => {
        const { screen } = await mountPlayer();

        // Click Play to start playback.
        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();

        // Do NOT resolve the fetch — the render loop will advance elapsed time
        // past the buffer and trigger the buffering overlay.
        const buffering = screen.getByText('Buffering...');
        await expect.element(buffering, { timeout: 5000 }).toBeVisible();
    });

    it('buffering overlay disappears when data arrives', async () => {
        const { screen, mockDataSource } = await mountPlayer();

        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();

        const buffering = screen.getByText('Buffering...');
        await expect.element(buffering, { timeout: 5000 }).toBeVisible();

        // Continuously resolve pending fetches until the buffering overlay disappears.
        // The rAF render loop may queue additional fetches as elapsed time advances,
        // so a single resolveFetch is not always sufficient.
        const drainInterval = setInterval(() => {
            while (mockDataSource.pendingCount > 0) {
                mockDataSource.resolveFetch(makePdus(0, 30_000));
            }
        }, 50);

        try {
            // The {#if} block removes the element from DOM entirely.
            // Use expect.poll to check that the element is gone.
            await expect.poll(
                () => screen.container.querySelector('.__irp-buffering-overlay'),
                { timeout: 5000 },
            ).toBeNull();
        } finally {
            clearInterval(drainInterval);
        }
    });

    it('action overlay appears on play/pause toggle', async () => {
        const { screen, mockDataSource } = await mountPlayer();

        // Keyboard shortcut ('k') — overlay blocks canvas click in test env.
        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();
        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true }));

        // Resolve the fetch so the store is happy.
        if (mockDataSource.pendingCount > 0) {
            mockDataSource.resolveFetch(makePdus(0, 15_000));
        }

        // The action overlay shows ▶ (play) or ⏸ (pause). It auto-dismisses after 600ms.
        // Assert immediately — the overlay is shown synchronously by showActionOverlay().
        const actionOverlay = screen.getByText('▶');
        await expect.element(actionOverlay).toBeVisible();
    });

    it('ended overlay appears when recording ends', async () => {
        const { screen, mockDataSource, mockWasm } = await mountPlayer();

        // Start playback.
        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();

        // Resolve fetch with data so playback can proceed.
        if (mockDataSource.pendingCount > 0) {
            mockDataSource.resolveFetch(makePdus(0, 15_000));
        }

        // Signal that the session has ended.
        mockWasm.setSessionEnded();

        const endedOverlay = screen.getByText('Replay');
        await expect.element(endedOverlay, { timeout: 5000 }).toBeVisible();
    });
});
