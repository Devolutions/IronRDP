import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RemoteDesktopService } from './remote-desktop.service';
import { ClipboardService } from './clipboard.service';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import type { ClipboardData } from '../interfaces/ClipboardData';
import { ConfigBuilder } from './ConfigBuilder';

/**
 * Clipboard monitoring and the clipboard write that waits for focus belong to each component. A page can show
 * several sessions; destroying one of them used to stop monitoring for all of them (a shared "destroyed" store),
 * and writes queued by any of them ran on the next focus (a shared queue).
 */

function textData(text: string): ClipboardData {
    return {
        items: () => [{ mimeType: () => 'text/plain', value: () => text }],
        isEmpty: () => false,
    } as unknown as ClipboardData;
}

const module = {
    ClipboardData: class {
        addText() {}
        addBinary() {}
        isEmpty() {
            return false;
        }
    },
} as unknown as RemoteDesktopModule;

async function started(): Promise<{ service: RemoteDesktopService; clipboard: ClipboardService }> {
    const service = new RemoteDesktopService(module);
    const clipboard = new ClipboardService(service, module);
    await clipboard.initClipboard();
    service.sessionStartedObservable.publish(null);
    return { service, clipboard };
}

// What the service hands a clipboard update from the server to (the auto-mode callback ClipboardService installed).
function remoteCopies(service: RemoteDesktopService, text: string) {
    (service as unknown as { onRemoteClipboardChanged: (data: ClipboardData) => void }).onRemoteClipboardChanged(
        textData(text),
    );
}

// Lets pending promises (clipboard reads and writes) settle; timers are fake.
const tick = () => vi.advanceTimersByTimeAsync(1);

describe('clipboard lifecycle', () => {
    let focused: boolean;
    let written: string[];
    let reads: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        focused = true;
        written = [];
        reads = vi.fn(async () => []);
        vi.spyOn(document, 'hasFocus').mockImplementation(() => focused);
        vi.stubGlobal('isSecureContext', true);
        // jsdom's Blob can't be read back; the service only wraps the text in one.
        vi.stubGlobal(
            'Blob',
            class {
                constructor(readonly parts: string[]) {}
            },
        );
        vi.stubGlobal(
            'ClipboardItem',
            class {
                constructor(readonly items: Record<string, { parts: string[] }>) {}
            },
        );
        vi.stubGlobal('navigator', {
            ...navigator,
            clipboard: {
                read: reads,
                write: vi.fn(async ([item]: [{ items: Record<string, { parts: string[] }> }]) => {
                    written.push(item.items['text/plain'].parts[0]);
                }),
                readText: vi.fn(),
                writeText: vi.fn(),
            },
            permissions: { query: vi.fn(async () => ({ state: 'granted' })) },
        });
    });

    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    it('keeps monitoring one session when another component is disposed', async () => {
        const one = await started();
        const two = await started();

        two.clipboard.dispose();
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);

        // Only the remaining session polls: about once per 100 ms.
        expect(reads.mock.calls.length).toBeGreaterThanOrEqual(3);
        expect(reads.mock.calls.length).toBeLessThanOrEqual(6);
        one.clipboard.dispose();
    });

    it('stops monitoring when the session ends, and starts again with the next one', async () => {
        const { service, clipboard } = await started();

        service.shutdown();
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).not.toHaveBeenCalled();

        service.sessionStartedObservable.publish(null);
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).toHaveBeenCalled();
        clipboard.dispose();
    });

    it('stops monitoring when the session ends on its own', async () => {
        let endSession!: () => void;
        const session = {
            desktopSize: () => ({ width: 800, height: 600 }),
            run: () =>
                new Promise((resolve) => {
                    endSession = () => resolve({ reason: () => 'ended' });
                }),
            shutdown: vi.fn(),
            releaseAllInputs: vi.fn(),
            invokeExtension: vi.fn(),
        };
        // A session builder whose setters all return it, and whose connect() gives the session above.
        const connecting = {
            ...module,
            SessionBuilder: class {
                constructor() {
                    const builder: object = new Proxy(this, {
                        get: (_target, name: string) => (name === 'connect' ? async () => session : () => builder),
                    });
                    return builder;
                }
            },
            DesktopSize: class {
                constructor(
                    public width: number,
                    public height: number,
                ) {}
            },
        } as unknown as RemoteDesktopModule;
        const service = new RemoteDesktopService(connecting);
        service.setCanvas(document.createElement('canvas'));
        const clipboard = new ClipboardService(service, connecting);
        await clipboard.initClipboard();
        const config = new ConfigBuilder()
            .withDestination('host:3389')
            .withProxyAddress('ws://proxy')
            .withAuthToken('token')
            .build();
        const running = (await service.connect(config)).run();

        reads.mockClear();
        await vi.advanceTimersByTimeAsync(300);
        expect(reads).toHaveBeenCalled();

        endSession();
        await running;
        reads.mockClear();
        await vi.advanceTimersByTimeAsync(500);
        expect(reads).not.toHaveBeenCalled();
        clipboard.dispose();
    });

    it('writes only the latest remote clipboard once the window has the focus again', async () => {
        const { service, clipboard } = await started();

        focused = false;
        remoteCopies(service, 'first');
        remoteCopies(service, 'second');
        await tick();
        expect(written).toEqual([]);

        focused = true;
        clipboard.flushFocused();
        await tick();
        expect(written).toEqual(['second']);
        clipboard.dispose();
    });

    it('drops a waiting write when its component is disposed', async () => {
        const { service, clipboard } = await started();

        focused = false;
        remoteCopies(service, 'stale');
        clipboard.dispose();
        focused = true;
        clipboard.flushFocused();
        await tick();

        expect(written).toEqual([]);
    });
});
