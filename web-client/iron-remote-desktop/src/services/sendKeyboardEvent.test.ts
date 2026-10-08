import { describe, it, expect, vi } from 'vitest';
import { PublicAPI } from './PublicAPI';
import type { RemoteDesktopService } from './remote-desktop.service';
import type { ClipboardService } from './clipboard.service';

/**
 * `sendKeyboardEvent` on the public API hands a key event to the session's own key path, the one the canvas uses
 * while it has the focus, so a host can send keys to a session that doesn't have it.
 */
describe('PublicAPI.sendKeyboardEvent', () => {
    it('passes the event to the session as the canvas would', () => {
        const service = { sendKeyboardEvent: vi.fn() } as unknown as RemoteDesktopService;
        const api = new PublicAPI(service, {} as ClipboardService).getExposedFunctions();
        const down = new KeyboardEvent('keydown', { code: 'KeyA', key: 'a' });
        const up = new KeyboardEvent('keyup', { code: 'KeyA', key: 'a' });

        api.sendKeyboardEvent(down);
        api.sendKeyboardEvent(up);

        expect(service.sendKeyboardEvent).toHaveBeenNthCalledWith(1, down);
        expect(service.sendKeyboardEvent).toHaveBeenNthCalledWith(2, up);
    });
});
