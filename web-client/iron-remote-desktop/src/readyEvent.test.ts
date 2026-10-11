import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, unmount } from 'svelte';
import IronRemoteDesktop from './iron-remote-desktop.svelte';
import type { RemoteDesktopModule } from './interfaces/RemoteDesktopModule';

/**
 * Regression test: the `ready` event must come after the clipboard is initialized.
 *
 * `connect()` only registers the clipboard callbacks on the session builder (and so only gets a clipboard
 * channel) when `ClipboardService.initClipboard()` has already installed them. `initClipboard()` awaits a
 * permissions query, so a host that connects as soon as `ready` fires used to get a session without clipboard.
 */

// Session builder that records which methods were called, and never actually connects.
function createRecordingModule(calls: string[]): RemoteDesktopModule {
    const SessionBuilder = class {
        constructor() {
            return new Proxy(this, {
                get: (_target, name: string) =>
                    name === 'connect'
                        ? () => Promise.reject(new Error('not connecting in tests'))
                        : () => {
                              calls.push(name);
                          },
            });
        }
    };
    return {
        SessionBuilder,
        DesktopSize: class {},
        InputTransaction: class {},
        ClipboardData: class {},
        DeviceEvent: {},
    } as unknown as RemoteDesktopModule;
}

function stubClipboardApi(permissionQuery: () => Promise<{ state: string }>) {
    vi.stubGlobal('isSecureContext', true);
    vi.stubGlobal('navigator', {
        ...navigator,
        clipboard: { read: vi.fn(), write: vi.fn(), readText: vi.fn(), writeText: vi.fn() },
        permissions: { query: vi.fn(permissionQuery) },
    });
}

describe('ready event', () => {
    let component: ReturnType<typeof mount> | null = null;

    afterEach(() => {
        if (component) {
            unmount(component);
            component = null;
        }
        vi.unstubAllGlobals();
        document.body.innerHTML = '';
    });

    it('fires only after the clipboard is initialized, so connecting on it opens the clipboard channel', async () => {
        let grantPermission!: () => void;
        stubClipboardApi(
            () =>
                new Promise((resolve) => {
                    grantPermission = () => resolve({ state: 'granted' });
                }),
        );

        const calls: string[] = [];
        const target = document.createElement('div');
        document.body.appendChild(target);
        const ready = vi.fn();
        target.addEventListener('ready', ready);

        component = mount(IronRemoteDesktop, {
            target,
            props: { scale: 'fit', verbose: 'false', flexcenter: 'true', module: createRecordingModule(calls) },
        });

        // The clipboard setup is waiting on the permission query: not ready yet.
        await vi.waitFor(() => expect(navigator.permissions.query).toHaveBeenCalled());
        await new Promise((resolve) => setTimeout(resolve, 20));
        expect(ready).not.toHaveBeenCalled();

        grantPermission();
        await vi.waitFor(() => expect(ready).toHaveBeenCalledTimes(1));

        // A host connecting right away gets the clipboard callbacks registered.
        const ui = (ready.mock.calls[0][0] as CustomEvent).detail.irgUserInteraction;
        const config = ui
            .configBuilder()
            .withDestination('host:3389')
            .withProxyAddress('ws://proxy')
            .withAuthToken('token')
            .build();
        await ui.connect(config).catch(() => undefined);
        expect(calls).toContain('remoteClipboardChangedCallback');
    });

    it('still fires when the clipboard is unavailable', async () => {
        vi.stubGlobal('isSecureContext', false);

        const target = document.createElement('div');
        document.body.appendChild(target);
        const ready = vi.fn();
        target.addEventListener('ready', ready);

        component = mount(IronRemoteDesktop, {
            target,
            props: { scale: 'fit', verbose: 'false', flexcenter: 'true', module: createRecordingModule([]) },
        });

        await vi.waitFor(() => expect(ready).toHaveBeenCalledTimes(1));
    });
});
