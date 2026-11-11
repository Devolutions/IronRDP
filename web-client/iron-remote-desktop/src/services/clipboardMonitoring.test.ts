import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RemoteDesktopService } from './remote-desktop.service';
import { ClipboardService } from './clipboard.service';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';

/**
 * A read rejected with NotAllowedError stops clipboard monitoring only while the page has the focus and the
 * `clipboard-read` permission isn't granted. Otherwise the rejection is transient (Chromium's "Document is not focused.").
 */

const module = {
    ClipboardData: class {
        addText() {}
        addBinary() {}
        isEmpty() {
            return false;
        }
    },
} as unknown as RemoteDesktopModule;

class FakePermissionStatus extends EventTarget {
    constructor(public state: PermissionState) {
        super();
    }

    change(state: PermissionState) {
        this.state = state;
        this.dispatchEvent(new Event('change'));
    }
}

const notAllowed = (message: string) => new DOMException(message, 'NotAllowedError');

describe('clipboard monitoring after NotAllowedError', () => {
    let permission: FakePermissionStatus;
    let query: ReturnType<typeof vi.fn>;
    let reads: ReturnType<typeof vi.fn>;

    async function started() {
        const service = new RemoteDesktopService(module);
        const clipboard = new ClipboardService(service, module);
        await clipboard.initClipboard();
        service.sessionStartedObservable.publish(null);
        return { service, clipboard };
    }

    beforeEach(() => {
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        vi.spyOn(document, 'hasFocus').mockReturnValue(true);
        vi.spyOn(console, 'warn').mockImplementation(() => {});
        vi.spyOn(console, 'error').mockImplementation(() => {});
        vi.stubGlobal('isSecureContext', true);
        permission = new FakePermissionStatus('granted');
        query = vi.fn(async () => permission);
        reads = vi.fn(async () => []);
        vi.stubGlobal('navigator', {
            ...navigator,
            clipboard: { read: reads, write: vi.fn(), readText: vi.fn(), writeText: vi.fn() },
            permissions: { query },
        });
    });

    afterEach(() => {
        vi.clearAllTimers();
        vi.useRealTimers();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    it('keeps monitoring when the permission is granted', async () => {
        await started();
        reads.mockRejectedValueOnce(notAllowed('Document is not focused.'));

        await vi.advanceTimersByTimeAsync(150);
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);

        expect(reads).toHaveBeenCalled();
        expect(console.warn).not.toHaveBeenCalled();
    });

    it('keeps monitoring when the page lost the focus', async () => {
        permission.state = 'prompt';
        await started();
        reads.mockImplementationOnce(async () => {
            vi.mocked(document.hasFocus).mockReturnValue(false);
            throw notAllowed('Document is not focused.');
        });

        await vi.advanceTimersByTimeAsync(150);
        vi.mocked(document.hasFocus).mockReturnValue(true);
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);

        expect(reads).toHaveBeenCalled();
        expect(console.warn).not.toHaveBeenCalled();
    });

    it('stops monitoring until the permission is granted', async () => {
        permission.state = 'prompt';
        await started();
        reads.mockRejectedValueOnce(notAllowed('Read permission denied.'));

        await vi.advanceTimersByTimeAsync(150);
        expect(console.warn).toHaveBeenCalledWith('Clipboard monitoring disabled: Read permission denied.');
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).not.toHaveBeenCalled();

        permission.change('denied');
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).not.toHaveBeenCalled();

        permission.change('granted');
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).toHaveBeenCalled();
    });

    it('does not restart monitoring once it was stopped', async () => {
        permission.state = 'prompt';
        const { service, clipboard } = await started();
        reads.mockRejectedValueOnce(notAllowed('Read permission denied.'));
        await vi.advanceTimersByTimeAsync(150);

        clipboard.stopMonitoring();
        const setCallback = vi.spyOn(service, 'setOnRemoteClipboardChanged');
        permission.change('granted');
        await vi.advanceTimersByTimeAsync(500);

        expect(setCallback).not.toHaveBeenCalled();
        expect(reads).toHaveBeenCalledTimes(1);
    });

    it('stops monitoring when the browser cannot tell the permission', async () => {
        await started();
        query.mockRejectedValue(new TypeError("'clipboard-read' is not a valid PermissionName"));
        reads.mockRejectedValueOnce(notAllowed('Clipboard read request was blocked due to lack of user activation.'));

        await vi.advanceTimersByTimeAsync(150);
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);

        expect(reads).not.toHaveBeenCalled();
        expect(console.warn).toHaveBeenCalled();
    });
});
