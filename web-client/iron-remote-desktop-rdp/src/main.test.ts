import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MonitorRect } from './main';

const captured = vi.hoisted(() => ({
    extensions: [] as Array<{ name: string; value: unknown }>,
}));

vi.mock('../../../crates/ironrdp-web/pkg/ironrdp_web', () => ({
    default: vi.fn(),
    setup: vi.fn(),
    DesktopSize: class {},
    DeviceEvent: class {},
    InputTransaction: class {},
    SessionBuilder: class {},
    ClipboardData: class {},
    RdpFile: class {},
    Extension: class {
        constructor(name: string, value: unknown) {
            captured.extensions.push({ name, value });
        }
    },
}));

import { monitorLayout, monitorLayoutCallback } from './main';

describe('monitor layout extensions', () => {
    beforeEach(() => {
        captured.extensions.length = 0;
    });

    it('round-trips monitor coordinates and primary status', () => {
        // oracle: spec MS-RDPBCGR §2.2.1.3.6.1 defines the coordinate order and primary flag.
        const monitors: MonitorRect[] = [
            { left: 0, top: 0, right: 799, bottom: 599, primary: true },
            { left: -800, top: 0, right: -1, bottom: 599, primary: false },
        ];
        const callback = vi.fn();

        monitorLayout(monitors);
        monitorLayoutCallback(callback);

        const request = captured.extensions[0];
        const response = captured.extensions[1];
        expect(request.name).toBe('monitor_layout');
        expect(request.value).toEqual([0, 0, 799, 599, 1, -800, 0, -1, 599, 0]);
        expect(response.name).toBe('monitor_layout_callback');
        if (typeof response.value !== 'function') {
            throw new Error('monitor callback extension must be a function');
        }
        response.value(request.value);
        expect(callback).toHaveBeenCalledWith(monitors);
    });

    it('exposes only the primary bit from server monitor flags', () => {
        // oracle: spec MS-RDPBCGR §2.2.1.3.6.1 defines 0x00000001 as the primary flag.
        const callback = vi.fn();
        monitorLayoutCallback(callback);
        const response = captured.extensions[0];
        if (typeof response.value !== 'function') {
            throw new Error('monitor callback extension must be a function');
        }
        response.value([0, 0, 799, 599, 0x80000001, -800, 0, -1, 599, 0x00000002]);
        expect(callback).toHaveBeenCalledWith([
            { left: 0, top: 0, right: 799, bottom: 599, primary: true },
            { left: -800, top: 0, right: -1, bottom: 599, primary: false },
        ]);
    });
});
