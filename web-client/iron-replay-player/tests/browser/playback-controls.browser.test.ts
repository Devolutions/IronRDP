import { describe, it, expect } from 'vitest';
import { page } from 'vitest/browser';
import { mountPlayer, mountPlayerPartial } from './setup.js';
import { makePdus } from '../helpers/mock-data-source.js';
import type { ReplayPdu } from '../../src/interfaces/ReplayDataSource.js';

type MockDS = Awaited<ReturnType<typeof mountPlayer>>['mockDataSource'];

/**
 * Wait for at least one pending fetch to appear, then resolve it.
 */
async function waitAndResolveFetch(ds: MockDS, pdus: ReplayPdu[] = []): Promise<void> {
    for (let i = 0; i < 200; i++) {
        if (ds.pendingCount > 0) {
            ds.resolveFetch(pdus);
            return;
        }
        await new Promise((r) => setTimeout(r, 10));
    }
    throw new Error('Timed out waiting for pending fetch');
}

/**
 * Drain all pending fetches until a promise settles.
 * Returns when the promise resolves.
 */
async function drainFetchesUntilSettled(
    ds: MockDS,
    promise: Promise<void>,
    pdus: ReplayPdu[] = [],
): Promise<void> {
    let settled = false;
    promise.then(
        () => { settled = true; },
        () => { settled = true; },
    );

    for (let i = 0; i < 500 && !settled; i++) {
        while (ds.pendingCount > 0) {
            ds.resolveFetch(pdus);
        }
        await new Promise((r) => setTimeout(r, 10));
    }

    // Final await to propagate any rejection.
    await promise;
}

describe('Playback controls', () => {
    it('play button starts playback', async () => {
        const { api, mockDataSource } = await mountPlayer();

        expect(api.isPaused()).toBe(true);

        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();

        // play() triggers a fetch — wait for it and resolve.
        await waitAndResolveFetch(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);
    });

    it('pause button pauses playback', async () => {
        const { api, mockDataSource } = await mountPlayer();

        // Start playing.
        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();
        await waitAndResolveFetch(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);

        // Click pause.
        const pauseBtn = page.getByRole('button', { name: 'Pause', exact: true });
        await pauseBtn.click();

        expect(api.isPaused()).toBe(true);
    });

    it('reset button seeks to 0', async () => {
        const { api, mockDataSource } = await mountPlayer();

        // Seek to 10_000 via api. Drain fetches until the seek completes.
        await drainFetchesUntilSettled(
            mockDataSource,
            api.seek(10_000),
        );

        expect(api.getElapsedMs()).toBe(10_000);

        // Click the Reset button. Reset calls seek(0) internally.
        const resetBtn = page.getByRole('button', { name: 'Reset to beginning' });
        await resetBtn.click();

        // seek(0) is a backward seek with no fetch chunks — resolves immediately.
        // Drain in case any fetches are pending.
        for (let i = 0; i < 50; i++) {
            while (mockDataSource.pendingCount > 0) {
                mockDataSource.resolveFetch([]);
            }
            await new Promise((r) => setTimeout(r, 10));
            if (api.getElapsedMs() === 0) break;
        }

        expect(api.getElapsedMs()).toBe(0);
    });

    it('canvas click toggles play/pause', async () => {
        const { api, mockDataSource, screen } = await mountPlayer();

        expect(api.isPaused()).toBe(true);

        // Synthetic click on container — overlay stops propagation
        // for physical clicks (default 300x150 canvas).
        const canvasContainer = screen.container.querySelector('.__irp-canvas-container')!;
        canvasContainer.dispatchEvent(new MouseEvent('click', { bubbles: true }));

        // Resolve the fetch triggered by play().
        await waitAndResolveFetch(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);

        // Click again to pause.
        canvasContainer.dispatchEvent(new MouseEvent('click', { bubbles: true }));

        expect(api.isPaused()).toBe(true);
    });

    it('ended overlay restarts playback', async () => {
        const { api, mockDataSource, mockWasm } = await mountPlayer();

        // Start playing.
        const playBtn = page.getByRole('button', { name: 'Play', exact: true });
        await playBtn.click();
        await waitAndResolveFetch(mockDataSource, makePdus(0, 15_000));

        // Trigger session ended — the rAF render loop will detect it.
        mockWasm.setSessionEnded();

        // Wait for the ended overlay to appear.
        const replayBtn = page.getByRole('button', { name: 'Replay from beginning' });
        await expect.element(replayBtn, { timeout: 5000 }).toBeVisible();

        // Reset render behavior so replay can restart.
        mockWasm.resetRenderBehavior();

        // Click the ended overlay.
        const overlayEl = document.querySelector('.__irp-ended-overlay');
        expect(overlayEl).not.toBeNull();
        const overlayLocator = page.elementLocator(overlayEl!);
        await overlayLocator.click();

        // The overlay click calls reset().then(() => play()).
        // Drain fetches until playback resumes.
        for (let i = 0; i < 100; i++) {
            while (mockDataSource.pendingCount > 0) {
                mockDataSource.resolveFetch(makePdus(0, 15_000));
            }
            if (!api.isPaused()) break;
            await new Promise((r) => setTimeout(r, 20));
        }

        expect(api.isPaused()).toBe(false);
        expect(api.getElapsedMs()).toBe(0);
    });

    it('error state shows error message', async () => {
        const { mockDataSource } = await mountPlayerPartial();

        // Reject the deferred open() call.
        mockDataSource.rejectOpen(new Error('connection refused'));

        // Wait for the error element to appear.
        const errorEl = page.getByText('Error: connection refused');
        await expect.element(errorEl, { timeout: 5000 }).toBeVisible();
    });

    it('interactions are no-op before WASM init', async () => {
        const { screen } = await mountPlayerPartial();

        // The player div should have tabindex="-1" since canPlay is false.
        const playerDiv = screen.container.querySelector('.__irp-replay-player');
        expect(playerDiv).not.toBeNull();
        const playerLocator = page.elementLocator(playerDiv!);
        await expect.element(playerLocator).toHaveAttribute('tabindex', '-1');
    });
});
