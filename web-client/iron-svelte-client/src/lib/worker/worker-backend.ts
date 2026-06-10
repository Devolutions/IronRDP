// Main-thread worker-backed backend: a drop-in `RemoteDesktopModule` whose session runs entirely
// in `rdp.worker.ts`. `RemoteDesktopService` is unchanged — it builds `DeviceEvent`s (plain
// descriptors here), an `InputTransaction` (a descriptor collector), and a `SessionBuilder`
// (`WorkerSessionBuilder`) whose `connect()` transfers the canvas to the worker via
// `transferControlToOffscreen()`. Cursor/clipboard live in the DOM, so cursor styles forwarded by
// the worker are re-applied here through the callback the service registered.

import type { ExtensionDescriptor, FromWorker, InputDescriptor, ToWorker, WorkerConnectConfig } from './worker-protocol';

type CursorCallback = (
    style: string,
    data: string | undefined,
    hotspotX: number | undefined,
    hotspotY: number | undefined,
) => void;

function spawnWorker(): Worker {
    // Vite bundles the worker and its wasm import as a separate module-worker chunk.
    return new Worker(new URL('./rdp.worker.ts', import.meta.url), { type: 'module' });
}

class WorkerDesktopSize {
    constructor(
        public width: number,
        public height: number,
    ) {}
}

class WorkerInputTransaction {
    events: InputDescriptor[] = [];
    addEvent(event: unknown): void {
        this.events.push(event as InputDescriptor);
    }
}

// Clipboard is not bridged in worker render mode yet; a minimal no-op satisfies the contract.
class WorkerClipboardData {
    addText(_mimeType: string, _text: string): void {}
    addBinary(_mimeType: string, _binary: Uint8Array): void {}
    items(): unknown[] {
        return [];
    }
    isEmpty(): boolean {
        return true;
    }
}

const WorkerDeviceEvent = {
    mouseButtonPressed: (button: number): InputDescriptor => ({ kind: 'mouseButtonPressed', button }),
    mouseButtonReleased: (button: number): InputDescriptor => ({ kind: 'mouseButtonReleased', button }),
    mouseMove: (x: number, y: number): InputDescriptor => ({ kind: 'mouseMove', x, y }),
    wheelRotations: (vertical: boolean, amount: number, unit: number): InputDescriptor => ({
        kind: 'wheelRotations',
        vertical,
        amount,
        unit,
    }),
    keyPressed: (scancode: number): InputDescriptor => ({ kind: 'keyPressed', scancode }),
    keyReleased: (scancode: number): InputDescriptor => ({ kind: 'keyReleased', scancode }),
    unicodePressed: (unicode: string): InputDescriptor => ({ kind: 'unicodePressed', unicode }),
    unicodeReleased: (unicode: string): InputDescriptor => ({ kind: 'unicodeReleased', unicode }),
};

class WorkerSession {
    private endResolve?: (info: { reason(): string }) => void;

    constructor(
        private worker: Worker,
        private cursorCb: CursorCallback | undefined,
        private cursorCtx: unknown,
        private desktop: { width: number; height: number },
    ) {
        worker.addEventListener('message', this.onMessage);
    }

    private onMessage = (ev: MessageEvent<FromWorker>) => {
        const m = ev.data;
        switch (m.type) {
            case 'cursor':
                this.cursorCb?.call(this.cursorCtx, m.style, m.data, m.hotspotX, m.hotspotY);
                break;
            case 'warning':
                console.warn('[rdp-worker]', m.message);
                break;
            case 'error':
                console.error('[rdp-worker] session error:', m.message);
                this.endResolve?.({ reason: () => m.message });
                break;
            case 'ended':
                this.endResolve?.({ reason: () => m.reason });
                break;
        }
    };

    desktopSize(): WorkerDesktopSize {
        return new WorkerDesktopSize(this.desktop.width, this.desktop.height);
    }

    run(): Promise<{ reason(): string }> {
        return new Promise((resolve) => {
            this.endResolve = resolve;
        });
    }

    applyInputs(transaction: WorkerInputTransaction): void {
        if (transaction.events.length > 0) {
            this.post({ type: 'input', events: transaction.events });
        }
    }

    releaseAllInputs(): void {
        this.post({ type: 'releaseAllInputs' });
    }

    synchronizeLockKeys(scrollLock: boolean, numLock: boolean, capsLock: boolean, kanaLock: boolean): void {
        this.post({ type: 'synchronizeLockKeys', scroll: scrollLock, num: numLock, caps: capsLock, kana: kanaLock });
    }

    resize(
        width: number,
        height: number,
        scaleFactor?: number | null,
        physicalWidth?: number | null,
        physicalHeight?: number | null,
    ): void {
        this.post({ type: 'resize', width, height, scaleFactor, physicalWidth, physicalHeight });
    }

    shutdown(): void {
        this.post({ type: 'shutdown' });
        this.worker.terminate();
    }

    // Clipboard / runtime extensions are not bridged in worker render mode yet.
    onClipboardPaste(_data: unknown): Promise<void> {
        return Promise.resolve();
    }

    supportsUnicodeKeyboardShortcuts(): boolean {
        return true;
    }

    invokeExtension(_ext: unknown): unknown {
        throw new Error('runtime extensions are not supported in worker render mode');
    }

    private post(message: ToWorker): void {
        this.worker.postMessage(message);
    }
}

class WorkerSessionBuilder {
    private cfg: WorkerConnectConfig = {
        username: '',
        destination: '',
        serverDomain: '',
        password: '',
        proxyAddress: '',
        authToken: '',
        extensions: [],
    };
    private canvas?: HTMLCanvasElement;
    private cursorCb?: CursorCallback;
    private cursorCtx: unknown;

    username(v: string): this {
        this.cfg.username = v;
        return this;
    }
    destination(v: string): this {
        this.cfg.destination = v;
        return this;
    }
    serverDomain(v: string): this {
        this.cfg.serverDomain = v;
        return this;
    }
    password(v: string): this {
        this.cfg.password = v;
        return this;
    }
    proxyAddress(v: string): this {
        this.cfg.proxyAddress = v;
        return this;
    }
    authToken(v: string): this {
        this.cfg.authToken = v;
        return this;
    }
    desktopSize(ds: { width: number; height: number }): this {
        this.cfg.desktopSize = { width: ds.width, height: ds.height };
        return this;
    }
    renderCanvas(canvas: HTMLCanvasElement): this {
        this.canvas = canvas;
        return this;
    }
    setCursorStyleCallback(cb: CursorCallback): this {
        this.cursorCb = cb;
        return this;
    }
    setCursorStyleCallbackContext(ctx: unknown): this {
        this.cursorCtx = ctx;
        return this;
    }
    // Clipboard/canvas-resize callbacks are not bridged yet; accepted and ignored.
    remoteClipboardChangedCallback(_cb: unknown): this {
        return this;
    }
    forceClipboardUpdateCallback(_cb: unknown): this {
        return this;
    }
    canvasResizedCallback(_cb: unknown): this {
        return this;
    }
    extension(ext: unknown): this {
        this.cfg.extensions.push(ext as ExtensionDescriptor);
        return this;
    }

    async connect(): Promise<WorkerSession> {
        if (this.canvas == null) {
            throw new Error('renderCanvas must be called before connect in worker mode');
        }
        const worker = spawnWorker();
        const offscreen = this.canvas.transferControlToOffscreen();

        const connected = new Promise<{ width: number; height: number }>((resolve, reject) => {
            const onMsg = (ev: MessageEvent<FromWorker>) => {
                if (ev.data.type === 'connected') {
                    worker.removeEventListener('message', onMsg);
                    resolve(ev.data.desktopSize);
                } else if (ev.data.type === 'error') {
                    worker.removeEventListener('message', onMsg);
                    reject(new Error(ev.data.message));
                }
            };
            worker.addEventListener('message', onMsg);
        });

        const connectMsg: ToWorker = { type: 'connect', config: this.cfg, canvas: offscreen };
        worker.postMessage(connectMsg, [offscreen]);

        const desktop = await connected;
        return new WorkerSession(worker, this.cursorCb, this.cursorCtx, desktop);
    }
}

/** Drop-in `RemoteDesktopModule` that routes the session through `rdp.worker.ts`. */
export const WorkerBackend = {
    DesktopSize: WorkerDesktopSize,
    InputTransaction: WorkerInputTransaction,
    SessionBuilder: WorkerSessionBuilder,
    ClipboardData: WorkerClipboardData,
    DeviceEvent: WorkerDeviceEvent,
};

/** In worker mode the wasm is initialized inside the worker on connect; nothing to do here. */
export async function workerInit(_logLevel: string): Promise<void> {}

/** Pre-connection extensions as serializable descriptors the worker rebuilds. */
export const workerRdpExtensions = {
    preConnectionBlob: (pcb: string): ExtensionDescriptor => ({ ident: 'pcb', value: pcb }),
    displayControl: (enable: boolean): ExtensionDescriptor => ({ ident: 'display_control', value: enable }),
    kdcProxyUrl: (url: string): ExtensionDescriptor => ({ ident: 'kdc_proxy_url', value: url }),
};
