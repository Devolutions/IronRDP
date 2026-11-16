import { describe, it, expect, vi } from 'vitest';
import { RemoteDesktopService } from './remote-desktop.service';
import { ClipboardService } from './clipboard.service';
import { PublicAPI } from './PublicAPI';
import type { Session } from '../interfaces/Session';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import type { Extension } from '../interfaces/Extension';

describe('invokeExtension', () => {
    const ext = { ident: 'stats', value: null } as unknown as Extension;
    const module = {} as RemoteDesktopModule;

    it('returns the session’s answer through the public API', () => {
        // oracle: regression #2454 (the public API surfaces what Session.invokeExtension returns)
        const session = { invokeExtension: vi.fn().mockReturnValue({ answered: true }) };
        const service = new RemoteDesktopService(module);
        service.session = session as unknown as Session;
        const api = new PublicAPI(service, new ClipboardService(service, module)).getExposedFunctions();

        expect(api.invokeExtension(ext)).toEqual({ answered: true });
        expect(session.invokeExtension).toHaveBeenCalledWith(ext);
    });

    it('returns undefined before a session exists', () => {
        // oracle: regression #2454 (no session, no answer)
        expect(new RemoteDesktopService(module).invokeExtension(ext)).toBeUndefined();
    });
});
