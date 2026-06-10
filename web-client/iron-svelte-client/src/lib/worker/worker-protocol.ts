// Message protocol between the main thread and the RDP render worker.
//
// The entire IronRDP session (WebSocket connect, decode, softblit/WebGPU present) runs in the
// worker against a transferred `OffscreenCanvas`, so the main thread (compositor + input + DOM)
// is never blocked by decode. Input descriptors are sent main->worker and rebuilt into wasm
// `DeviceEvent`s there; cursor/warning/lifecycle signals come back worker->main.

/** A pre-connection extension, reduced to a serializable `{ident, value}` the worker rebuilds. */
export interface ExtensionDescriptor {
    ident: string;
    value: unknown;
}

export interface WorkerConnectConfig {
    username: string;
    destination: string;
    serverDomain: string;
    password: string;
    proxyAddress: string;
    authToken: string;
    desktopSize?: { width: number; height: number };
    extensions: ExtensionDescriptor[];
}

/** Serializable input event; rebuilt into a wasm `DeviceEvent` in the worker. */
export type InputDescriptor =
    | { kind: 'mouseButtonPressed'; button: number }
    | { kind: 'mouseButtonReleased'; button: number }
    | { kind: 'mouseMove'; x: number; y: number }
    | { kind: 'wheelRotations'; vertical: boolean; amount: number; unit: number }
    | { kind: 'keyPressed'; scancode: number }
    | { kind: 'keyReleased'; scancode: number }
    | { kind: 'unicodePressed'; unicode: string }
    | { kind: 'unicodeReleased'; unicode: string };

export type ToWorker =
    | { type: 'connect'; config: WorkerConnectConfig; canvas: OffscreenCanvas }
    | { type: 'input'; events: InputDescriptor[] }
    | { type: 'releaseAllInputs' }
    | { type: 'synchronizeLockKeys'; scroll: boolean; num: boolean; caps: boolean; kana: boolean }
    | {
          type: 'resize';
          width: number;
          height: number;
          scaleFactor?: number | null;
          physicalWidth?: number | null;
          physicalHeight?: number | null;
      }
    | { type: 'shutdown' };

export type FromWorker =
    | { type: 'connected'; desktopSize: { width: number; height: number } }
    | { type: 'cursor'; style: string; data?: string; hotspotX?: number; hotspotY?: number }
    | { type: 'warning'; message: string }
    | { type: 'ended'; reason: string }
    | { type: 'error'; message: string };
