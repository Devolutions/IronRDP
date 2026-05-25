import { describe, it, expect } from 'vitest';
import { page } from 'vitest/browser';
import { mountPlayer } from './setup.js';

describe('Speed selector', () => {
    it('speed button opens popup', async () => {
        await mountPlayer();

        const speedBtn = page.getByRole('button', { name: 'Playback speed' });
        await speedBtn.click();

        await expect.element(page.getByRole('menu')).toBeVisible();
    });

    it('selecting a speed updates playback speed', async () => {
        const { api } = await mountPlayer();

        // Open popup
        await page.getByRole('button', { name: 'Playback speed' }).click();
        await expect.element(page.getByRole('menu')).toBeVisible();

        // Direct DOM click — Playwright misses absolutely-positioned popups.
        const speed2 = page.getByRole('menuitem', { name: '2', exact: true });
        (speed2.element() as HTMLElement).click();
        await new Promise((r) => setTimeout(r, 50));

        expect(api.getSpeed()).toBe(2);
    });

    it('popup closes after speed selection', async () => {
        await mountPlayer();

        // Open popup
        await page.getByRole('button', { name: 'Playback speed' }).click();
        await expect.element(page.getByRole('menu')).toBeVisible();

        // Select a speed
        (page.getByRole('menuitem', { name: '2', exact: true }).element() as HTMLElement).click();
        await new Promise((r) => setTimeout(r, 50));

        // Popup should be gone (conditionally rendered with {#if})
        expect(page.getByRole('menu').query()).toBeNull();
    });

    it('click outside closes speed popup', async () => {
        await mountPlayer();

        // Open popup
        await page.getByRole('button', { name: 'Playback speed' }).click();
        await expect.element(page.getByRole('menu')).toBeVisible();

        // Click outside the popup (the Play button is outside the speed selector)
        await page.getByRole('button', { name: 'Play', exact: true }).click();
        await new Promise((r) => setTimeout(r, 50));

        // Popup should be gone
        expect(page.getByRole('menu').query()).toBeNull();
    });
});
