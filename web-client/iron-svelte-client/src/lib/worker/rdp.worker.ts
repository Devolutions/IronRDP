/// <reference lib="webworker" />
//
// RDP render worker: hosts the entire IronRDP wasm session (WebSocket connect, decode, softblit
// WebGPU present) off the main thread, drawing into a transferred `OffscreenCanvas`. The main
// thread keeps only the DOM, input capture, and cursor application. See `worker-protocol.ts`.

import {
    init,
    Backend,
    preConnectionBlob,
    displayControl,
    kdcProxyUrl,
    enableCredssp,
    outboundMessageSizeLimit,
} from '../../../static/iron-remote-desktop-rdp';
import type { ExtensionDescriptor, FromWorker, InputDescriptor, ToWorker } from './worker-protocol';

const ctx = self as unknown as DedicatedWorkerGlobalScope;

function post(message: FromWorker) {
    ctx.postMessage(message);
}

// Rebuild a serialized extension into a wasm `Extension` via the matching factory.
const EXTENSION_FACTORIES: Record<string, (value: never) => unknown> = {
    pcb: preConnectionBlob as (value: never) => unknown,
    display_control: displayControl as (value: never) => unknown,
    kdc_proxy_url: kdcProxyUrl as (value: never) => unknown,
    enable_credssp: enableCredssp as (value: never) => unknown,
    outbound_message_size_limit: outboundMessageSizeLimit as (value: never) => unknown,
};

function rebuildExtension(desc: ExtensionDescriptor): unknown {
    const factory = EXTENSION_FACTORIES[desc.ident];
    if (factory == null) {
        throw new Error(`unknown extension ident in worker: ${desc.ident}`);
    }
    return factory(desc.value as never);
}

function toDeviceEvent(d: InputDescriptor): unknown {
    const DE = Backend.DeviceEvent;
    switch (d.kind) {
        case 'mouseButtonPressed':
            return DE.mouseButtonPressed(d.button);
        case 'mouseButtonReleased':
            return DE.mouseButtonReleased(d.button);
        case 'mouseMove':
            return DE.mouseMove(d.x, d.y);
        case 'wheelRotations':
            return DE.wheelRotations(d.vertical, d.amount, d.unit);
        case 'keyPressed':
            return DE.keyPressed(d.scancode);
        case 'keyReleased':
            return DE.keyReleased(d.scancode);
        case 'unicodePressed':
            return DE.unicodePressed(d.unicode);
        case 'unicodeReleased':
            return DE.unicodeReleased(d.unicode);
    }
}

// The session object is opaque (wasm-bindgen). We only call a handful of methods on it.
interface WasmSession {
    desktopSize(): { width: number; height: number };
    run(): Promise<{ reason(): string }>;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    applyInputs(transaction: any): void;
    releaseAllInputs(): void;
    synchronizeLockKeys(scroll: boolean, num: boolean, caps: boolean, kana: boolean): void;
    resize(
        width: number,
        height: number,
        scaleFactor?: number | null,
        physicalWidth?: number | null,
        physicalHeight?: number | null,
    ): void;
    shutdown(): void;
}

let session: WasmSession | undefined;
let wasmReady: Promise<void> | undefined;

async function connect(config: ToWorker & { type: 'connect' }) {
    if (wasmReady == null) {
        wasmReady = init('INFO');
    }
    await wasmReady;

    const builder = new Backend.SessionBuilder()
        .proxyAddress(config.config.proxyAddress)
        .destination(config.config.destination)
        .serverDomain(config.config.serverDomain)
        .password(config.config.password)
        .authToken(config.config.authToken)
        .username(config.config.username)
        // The worker presents into the transferred OffscreenCanvas (GPU-only path).
        .renderOffscreenCanvas(config.canvas)
        // Cursor lives in the DOM (main thread); forward the style and re-apply it there.
        .setCursorStyleCallbackContext({})
        .setCursorStyleCallback(
            (style: string, data?: string, hotspotX?: number, hotspotY?: number) => {
                post({ type: 'cursor', style, data, hotspotX, hotspotY });
            },
        );

    for (const ext of config.config.extensions) {
        builder.extension(rebuildExtension(ext) as Parameters<typeof builder.extension>[0]);
    }

    if (config.config.desktopSize != null) {
        builder.desktopSize(new Backend.DesktopSize(config.config.desktopSize.width, config.config.desktopSize.height));
    }

    const built = (await builder.connect()) as unknown as WasmSession;
    session = built;

    const size = built.desktopSize();
    post({ type: 'connected', desktopSize: { width: size.width, height: size.height } });

    const info = await built.run();
    post({ type: 'ended', reason: info.reason() });
    session = undefined;
}

function applyInputs(events: InputDescriptor[]) {
    if (session == null) {
        return;
    }
    const transaction = new Backend.InputTransaction();
    for (const d of events) {
        transaction.addEvent(toDeviceEvent(d) as Parameters<typeof transaction.addEvent>[0]);
    }
    session.applyInputs(transaction);
}

ctx.onmessage = (event: MessageEvent<ToWorker>) => {
    const msg = event.data;
    try {
        switch (msg.type) {
            case 'connect':
                void connect(msg).catch((e: unknown) => {
                    post({ type: 'error', message: e instanceof Error ? e.message : String(e) });
                });
                break;
            case 'input':
                applyInputs(msg.events);
                break;
            case 'releaseAllInputs':
                session?.releaseAllInputs();
                break;
            case 'synchronizeLockKeys':
                session?.synchronizeLockKeys(msg.scroll, msg.num, msg.caps, msg.kana);
                break;
            case 'resize':
                session?.resize(msg.width, msg.height, msg.scaleFactor, msg.physicalWidth, msg.physicalHeight);
                break;
            case 'shutdown':
                session?.shutdown();
                break;
        }
    } catch (e: unknown) {
        post({ type: 'error', message: e instanceof Error ? e.message : String(e) });
    }
};
