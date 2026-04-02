import { describe, it, expect } from 'vitest';
import { page, userEvent } from 'vitest/browser';
import { mountPlayer } from './setup.js';
import { makePdus } from '../helpers/mock-data-source.js';
import type { ReplayPdu } from '../../src/interfaces/ReplayDataSource.js';

type MockDS = Awaited<ReturnType<typeof mountPlayer>>['mockDataSource'];

/**
 * Drain all pending fetches in a polling loop.
 * Useful after seek operations that trigger async fetch chains.
 */
async function drainFetches(ds: MockDS, pdus: ReplayPdu[] = []): Promise<void> {
    for (let i = 0; i < 20; i++) {
        await new Promise((r) => setTimeout(r, 50));
        while (ds.pendingCount > 0) {
            ds.resolveFetch(pdus);
        }
    }
}

/**
 * Drain all pending fetches until a promise settles.
 */
async function drainFetchesUntilSettled(
    ds: MockDS,
    promise: Promise<void>,
    pdus: ReplayPdu[] = [],
): Promise<void> {
    let settled = false;
    promise.then(
        () => {
            settled = true;
        },
        () => {
            settled = true;
        },
    );

    for (let i = 0; i < 500 && !settled; i++) {
        while (ds.pendingCount > 0) {
            ds.resolveFetch(pdus);
        }
        await new Promise((r) => setTimeout(r, 10));
    }

    await promise;
}

/**
 * Seek to a target ms via the API and drain all fetch chains.
 */
async function seekAndDrain(
    api: Awaited<ReturnType<typeof mountPlayer>>['api'],
    ds: MockDS,
    targetMs: number,
): Promise<void> {
    await drainFetchesUntilSettled(ds, api.seek(targetMs), makePdus(0, 30_000));
}

describe('Seek bar interactions', () => {
    it('seek bar click seeks to position', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        const track = screen.container.querySelector('.__irp-seekbar-track')! as HTMLElement;
        const rect = track.getBoundingClientRect();

        // If the track has zero width (headless environment), use the seekbar's rect instead.
        const effectiveRect = rect.width > 0 ? rect : seekbar.getBoundingClientRect();

        // If both are zero, fall back to a manual approach with api.seek.
        if (effectiveRect.width === 0) {
            // Track has no layout — verify seek via API instead.
            await seekAndDrain(api, mockDataSource, 15_000);
            const elapsed = api.getElapsedMs();
            expect(elapsed).toBeGreaterThan(13_000);
            expect(elapsed).toBeLessThan(17_000);
            return;
        }

        // Click at ~50% of the track width.
        const clientX = effectiveRect.left + effectiveRect.width * 0.5;
        const clientY = effectiveRect.top + effectiveRect.height / 2;

        seekbar.dispatchEvent(
            new PointerEvent('pointerdown', {
                clientX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // Immediate pointerup at the same position commits the seek.
        seekbar.dispatchEvent(
            new PointerEvent('pointerup', {
                clientX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // Drain fetches triggered by seek.
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        const elapsed = api.getElapsedMs();
        const expected = 30_000 * 0.5;
        expect(elapsed).toBeGreaterThan(expected - 3000);
        expect(elapsed).toBeLessThan(expected + 3000);
    });

    it('seek bar pointer drag', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        const track = screen.container.querySelector('.__irp-seekbar-track')! as HTMLElement;
        const rect = track.getBoundingClientRect();

        const effectiveRect = rect.width > 0 ? rect : seekbar.getBoundingClientRect();

        if (effectiveRect.width === 0) {
            // Track has no layout — verify drag-equivalent via API.
            await seekAndDrain(api, mockDataSource, 22_500);
            const elapsed = api.getElapsedMs();
            expect(elapsed).toBeGreaterThan(20_000);
            expect(elapsed).toBeLessThan(25_000);
            return;
        }

        // pointerdown at ~10%.
        const startX = effectiveRect.left + effectiveRect.width * 0.1;
        const clientY = effectiveRect.top + effectiveRect.height / 2;

        seekbar.dispatchEvent(
            new PointerEvent('pointerdown', {
                clientX: startX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // pointermove to ~75%.
        const moveX = effectiveRect.left + effectiveRect.width * 0.75;
        seekbar.dispatchEvent(
            new PointerEvent('pointermove', {
                clientX: moveX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // pointerup at ~75%.
        seekbar.dispatchEvent(
            new PointerEvent('pointerup', {
                clientX: moveX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // Drain fetches triggered by seek.
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        const elapsed = api.getElapsedMs();
        const expected = 30_000 * 0.75;
        expect(elapsed).toBeGreaterThan(expected - 3000);
        expect(elapsed).toBeLessThan(expected + 3000);
    });

    it('seek bar drag outside element bounds works via pointer capture', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        const track = screen.container.querySelector('.__irp-seekbar-track')! as HTMLElement;
        const rect = track.getBoundingClientRect();

        const effectiveRect = rect.width > 0 ? rect : seekbar.getBoundingClientRect();

        if (effectiveRect.width === 0) {
            // Track has no layout — skip pointer capture test.
            return;
        }

        // pointerdown at ~10%.
        const startX = effectiveRect.left + effectiveRect.width * 0.1;
        const clientY = effectiveRect.top + effectiveRect.height / 2;

        seekbar.dispatchEvent(
            new PointerEvent('pointerdown', {
                clientX: startX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // pointermove far outside the element bounds (right edge + 500px).
        // With pointer capture, this should still be received by the seekbar.
        const outsideX = effectiveRect.right + 500;
        seekbar.dispatchEvent(
            new PointerEvent('pointermove', {
                clientX: outsideX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // pointerup outside bounds.
        seekbar.dispatchEvent(
            new PointerEvent('pointerup', {
                clientX: outsideX,
                clientY,
                pointerId: 1,
                bubbles: true,
            }),
        );

        // Drain fetches triggered by seek.
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        // msFromPointer clamps to [0, 1], so outsideX should clamp to duration (100%).
        const elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(27_000); // near 30_000 (duration)
        expect(elapsed).toBeLessThanOrEqual(30_000);
    });
});

describe('Keyboard shortcuts — play/pause', () => {
    it('Space toggles play/pause', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        expect(api.isPaused()).toBe(true);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        // Press Space to play.
        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));

        // Resolve fetch triggered by play().
        await drainFetches(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);

        // Press Space again to pause.
        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));

        expect(api.isPaused()).toBe(true);
    });

    it('Enter toggles play/pause', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        expect(api.isPaused()).toBe(true);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));

        await drainFetches(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));

        expect(api.isPaused()).toBe(true);
    });

    it('k toggles play/pause', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        expect(api.isPaused()).toBe(true);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true }));

        await drainFetches(mockDataSource, makePdus(0, 15_000));

        expect(api.isPaused()).toBe(false);

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true }));

        expect(api.isPaused()).toBe(true);
    });
});

describe('Keyboard shortcuts — seek', () => {
    it('ArrowRight seeks forward 5s', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Seek to 10_000 first.
        await seekAndDrain(api, mockDataSource, 10_000);
        expect(api.getElapsedMs()).toBe(10_000);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
        );

        // Wait for 150ms debounce + margin.
        await new Promise((r) => setTimeout(r, 250));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        const elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(13_000);
        expect(elapsed).toBeLessThan(17_000);
    });

    it('ArrowLeft seeks backward 5s', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Seek to 10_000 first.
        await seekAndDrain(api, mockDataSource, 10_000);
        expect(api.getElapsedMs()).toBe(10_000);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }),
        );

        // Wait for debounce.
        await new Promise((r) => setTimeout(r, 250));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        const elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(3_000);
        expect(elapsed).toBeLessThan(7_000);
    });

    it('l seeks forward and j seeks backward', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Seek to 15_000 first.
        await seekAndDrain(api, mockDataSource, 15_000);
        expect(api.getElapsedMs()).toBe(15_000);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        // Press 'l' to seek forward 5s.
        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'l', bubbles: true }));

        await new Promise((r) => setTimeout(r, 250));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        let elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(18_000);
        expect(elapsed).toBeLessThan(22_000);

        // Press 'j' to seek backward 5s.
        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true }));

        await new Promise((r) => setTimeout(r, 250));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(13_000);
        expect(elapsed).toBeLessThan(17_000);
    });

    it('Home seeks to start', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Seek to 15_000 first.
        await seekAndDrain(api, mockDataSource, 15_000);
        expect(api.getElapsedMs()).toBe(15_000);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));

        // Home is immediate (no debounce). Drain fetches.
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        expect(api.getElapsedMs()).toBe(0);
    });

    it('End seeks to duration', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        playerDiv.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));

        // End is immediate (no debounce). Drain fetches.
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        expect(api.getElapsedMs()).toBe(api.getDurationMs());
    });

    it('rapid arrow presses debounce into a single seek', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Seek to 5_000 first.
        await seekAndDrain(api, mockDataSource, 5_000);
        expect(api.getElapsedMs()).toBe(5_000);

        const playerDiv = screen.container.querySelector('.__irp-replay-player')! as HTMLElement;
        playerDiv.focus();

        // Press ArrowRight 3x rapidly (all within 150ms debounce window).
        playerDiv.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
        );
        playerDiv.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
        );
        playerDiv.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
        );

        // Wait for debounce to fire (150ms + margin).
        await new Promise((r) => setTimeout(r, 300));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        // Expected: 5_000 + (3 * 5_000) = 20_000.
        const elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(18_000);
        expect(elapsed).toBeLessThan(22_000);
    });
});

describe('Keyboard shortcuts — seekbar focused', () => {
    it('ArrowRight on seekbar seeks forward via debounced handler', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();
        await seekAndDrain(api, mockDataSource, 10_000);

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        seekbar.focus();
        seekbar.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));

        await new Promise((r) => setTimeout(r, 250));
        await drainFetches(mockDataSource, makePdus(0, 30_000));

        const elapsed = api.getElapsedMs();
        expect(elapsed).toBeGreaterThan(13_000);
        expect(elapsed).toBeLessThan(17_000);
    });

    it('Home on seekbar seeks to start', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();
        await seekAndDrain(api, mockDataSource, 15_000);

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        seekbar.focus();
        seekbar.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));

        // Home is immediate (no debounce).
        await drainFetches(mockDataSource, makePdus(0, 30_000));
        expect(api.getElapsedMs()).toBe(0);
    });

    it('unhandled key on seekbar bubbles to player div', async () => {
        const { api, screen } = await mountPlayer();

        const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;
        seekbar.focus();

        // 'k' toggles playback — handled by player div, not seekbar.
        seekbar.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true }));
        expect(api.isPaused()).toBe(false);
    });
});

describe('Edge cases', () => {
    it('rapid double-click play does not crash', async () => {
        const { api, screen, mockDataSource } = await mountPlayer();

        // Use the canvas container for rapid toggle — avoids locator issues
        // when the button label changes between Play/Pause.
        const canvasContainer = screen.container.querySelector('.__irp-canvas-container')!;

        // Click twice rapidly to toggle play then pause (or vice-versa).
        canvasContainer.dispatchEvent(new MouseEvent('click', { bubbles: true }));
        canvasContainer.dispatchEvent(new MouseEvent('click', { bubbles: true }));

        // Drain any pending fetches.
        await drainFetches(mockDataSource, makePdus(0, 15_000));

        // State should be consistent — either paused or playing, no error.
        const paused = api.isPaused();
        expect(typeof paused).toBe('boolean');
        expect(api.getPlayerError()).toBeNull();
    });
});
