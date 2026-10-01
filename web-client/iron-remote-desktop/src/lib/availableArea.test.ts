import { describe, expect, it } from 'vitest';
import { availableAreaCorner, windowCorner } from './availableArea';

function hostWithBox(box: Partial<DOMRect>): Element {
    const host = document.createElement('div');
    host.getBoundingClientRect = () =>
        ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, ...box }) as DOMRect;
    return host;
}

describe('availableAreaCorner', () => {
    it("uses the host element's box when the host is smaller than the window", () => {
        // e.g. the left half of a split pane
        const host = hostWithBox({ left: 0, top: 40, right: 600, bottom: 700, width: 600, height: 660 });
        expect(availableAreaCorner(host)).toEqual({ x: 600, y: 700 });
    });

    it('falls back to the window when the host has no size yet', () => {
        expect(availableAreaCorner(hostWithBox({}))).toEqual(windowCorner());
    });

    it('falls back to the window without a host', () => {
        expect(availableAreaCorner(undefined)).toEqual(windowCorner());
        expect(windowCorner()).toEqual({ x: window.innerWidth, y: window.innerHeight });
    });
});
