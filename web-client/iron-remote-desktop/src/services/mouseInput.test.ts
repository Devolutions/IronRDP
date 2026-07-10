import { describe, it, expect, beforeEach, vi } from 'vitest';
import { RemoteDesktopService } from './remote-desktop.service';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import type { Session } from '../interfaces/Session';

/**
 * Regression tests for the Firefox stuck right-click bug.
 *
 * Root cause: The RDP server received mouseButtonPressed(2) but never
 * mouseButtonReleased(2) when the user right-clicked and then moved the
 * cursor off the canvas before releasing.  Two defects contributed:
 *
 *   Defect 1 (iron-remote-desktop.svelte): onmouseleave sent a spurious
 *     mouseButtonReleased for a hardcoded button index instead of calling
 *     releaseAllInputs — fixed in the component.
 *
 *   Defect 2 (remote-desktop.service.ts): mouseIn() did not reconcile the
 *     browser's event.buttons bitmask against the RDP session's assumed
 *     button state, so re-entering the canvas left stale "button held"
 *     state on the server — fixed by the mouseIn() implementation tested here.
 *
 * These tests also cover the mouseOut() path which must call releaseAllInputs.
 */

// ── Helpers ──────────────────────────────────────────────────────────────────

class MockInputTransaction {
    addEvent = vi.fn();
}

function createMockModule(): RemoteDesktopModule {
    return {
        SessionBuilder: class {} as unknown as RemoteDesktopModule['SessionBuilder'],
        DesktopSize: class {} as unknown as RemoteDesktopModule['DesktopSize'],
        InputTransaction: MockInputTransaction as unknown as RemoteDesktopModule['InputTransaction'],
        ClipboardData: class {} as unknown as RemoteDesktopModule['ClipboardData'],
        DeviceEvent: {
            mouseButtonPressed: vi.fn((id: number) => ({ type: 'pressed', id })),
            mouseButtonReleased: vi.fn((id: number) => ({ type: 'released', id })),
            mouseMove: vi.fn(),
            wheelRotations: vi.fn(),
            keyPressed: vi.fn(),
            keyReleased: vi.fn(),
            unicodePressed: vi.fn(),
            unicodeReleased: vi.fn(),
        },
    };
}

function createMockSession(): Session {
    return {
        run: vi.fn().mockResolvedValue({ reason: () => 'test' }),
        desktopSize: vi.fn().mockReturnValue({ width: 1920, height: 1080 }),
        applyInputs: vi.fn(),
        releaseAllInputs: vi.fn(),
        synchronizeLockKeys: vi.fn(),
        shutdown: vi.fn(),
        onClipboardPaste: vi.fn(),
        resize: vi.fn(),
        supportsUnicodeKeyboardShortcuts: vi.fn().mockReturnValue(false),
        invokeExtension: vi.fn(),
    } as unknown as Session;
}

// ── mouseOut ─────────────────────────────────────────────────────────────────

describe('mouseOut', () => {
    let service: RemoteDesktopService;
    let session: Session;

    beforeEach(() => {
        vi.clearAllMocks();
        service = new RemoteDesktopService(createMockModule());
        session = createMockSession();
        service.session = session;
    });

    it('calls releaseAllInputs on the session', () => {
        service.mouseOut(new MouseEvent('mouseleave'));
        expect(session.releaseAllInputs).toHaveBeenCalledTimes(1);
    });

    it('does not throw when there is no active session', () => {
        service.session = undefined;
        expect(() => service.mouseOut(new MouseEvent('mouseleave'))).not.toThrow();
    });
});

// ── focusLost ─────────────────────────────────────────────────────────────────

describe('focusLost', () => {
    let service: RemoteDesktopService;
    let session: Session;

    beforeEach(() => {
        vi.clearAllMocks();
        service = new RemoteDesktopService(createMockModule());
        session = createMockSession();
        service.session = session;
    });

    it('calls releaseAllInputs on the session', () => {
        service.focusLost();
        expect(session.releaseAllInputs).toHaveBeenCalledTimes(1);
    });

    it('does not throw when there is no active session', () => {
        service.session = undefined;
        expect(() => service.focusLost()).not.toThrow();
    });
});

// ── mouseIn button reconciliation ─────────────────────────────────────────────

describe('mouseIn button reconciliation', () => {
    let service: RemoteDesktopService;
    let mockModule: RemoteDesktopModule;
    let session: Session;

    beforeEach(() => {
        vi.clearAllMocks();
        mockModule = createMockModule();
        service = new RemoteDesktopService(mockModule);
        session = createMockSession();
        service.session = session;
    });

    function mouseIn(buttons: number) {
        service.mouseIn(new MouseEvent('mouseenter', { buttons }));
    }

    it('releases all three buttons when no buttons are physically held (buttons=0)', () => {
        mouseIn(0);
        const released = vi.mocked(mockModule.DeviceEvent.mouseButtonReleased);
        expect(released).toHaveBeenCalledWith(0); // left
        expect(released).toHaveBeenCalledWith(2); // right
        expect(released).toHaveBeenCalledWith(1); // middle
        expect(released).toHaveBeenCalledTimes(3);
    });

    it('does not release the right button when it is physically held (buttons=2)', () => {
        mouseIn(2);
        const released = vi.mocked(mockModule.DeviceEvent.mouseButtonReleased);
        expect(released).toHaveBeenCalledWith(0); // left released
        expect(released).toHaveBeenCalledWith(1); // middle released
        expect(released).not.toHaveBeenCalledWith(2); // right NOT released
        expect(released).toHaveBeenCalledTimes(2);
    });

    it('does not release the left button when it is physically held (buttons=1)', () => {
        mouseIn(1);
        const released = vi.mocked(mockModule.DeviceEvent.mouseButtonReleased);
        expect(released).toHaveBeenCalledWith(2); // right released
        expect(released).toHaveBeenCalledWith(1); // middle released
        expect(released).not.toHaveBeenCalledWith(0); // left NOT released
        expect(released).toHaveBeenCalledTimes(2);
    });

    it('does not release the middle button when it is physically held (buttons=4)', () => {
        mouseIn(4);
        const released = vi.mocked(mockModule.DeviceEvent.mouseButtonReleased);
        expect(released).toHaveBeenCalledWith(0); // left released
        expect(released).toHaveBeenCalledWith(2); // right released
        expect(released).not.toHaveBeenCalledWith(1); // middle NOT released
        expect(released).toHaveBeenCalledTimes(2);
    });

    it('releases no buttons when all three are physically held (buttons=7)', () => {
        mouseIn(7);
        expect(vi.mocked(mockModule.DeviceEvent.mouseButtonReleased)).not.toHaveBeenCalled();
    });

    it('does nothing when there is no active session (buttons=0)', () => {
        service.session = undefined;
        mouseIn(0);
        expect(vi.mocked(mockModule.DeviceEvent.mouseButtonReleased)).not.toHaveBeenCalled();
        expect(session.applyInputs).not.toHaveBeenCalled();
    });

    it('sends all releases in a single applyInputs transaction', () => {
        mouseIn(0); // all three released → 1 batched transaction
        expect(session.applyInputs).toHaveBeenCalledTimes(1);
    });

    it('sends no transactions when all buttons are held', () => {
        mouseIn(7);
        expect(session.applyInputs).not.toHaveBeenCalled();
    });
});
