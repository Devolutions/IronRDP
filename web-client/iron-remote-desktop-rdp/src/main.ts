import wasm_init, {
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    SessionBuilder,
    ClipboardData,
    Extension,
    RdpFile,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';

export async function init(log_level: string) {
    await wasm_init();
    setup(log_level);
}

export { RdpFile };

export const Backend = {
    DesktopSize: DesktopSize,
    InputTransaction: InputTransaction,
    SessionBuilder: SessionBuilder,
    ClipboardData: ClipboardData,
    DeviceEvent: DeviceEvent,
};

// --- Pre-connection configuration extensions ---

export function preConnectionBlob(pcb: string): Extension {
    return new Extension('pcb', pcb);
}

export type VmConnectMode = 'enhanced' | 'basic';

export function vmConnect(vmId: string, mode: VmConnectMode = 'enhanced'): Extension {
    if (vmId.trim() === '') {
        throw new Error('vmconnect requires a VM ID');
    }

    const payload = mode === 'enhanced' ? `${vmId};EnhancedMode=1` : vmId;
    return new Extension('vmconnect', payload);
}

export function displayControl(enable: boolean): Extension {
    return new Extension('display_control', enable);
}

/**
 * Signs in with Microsoft Entra ID (RDS AAD Auth) instead of CredSSP.
 * The callback receives the nonce the server sent after the TLS handshake and returns the compact
 * JWS RDP assertion, or a promise of it; `signEntraAssertion` builds one.
 * The username and password are not needed, because the server takes the user from the access token.
 */
export function entraAssertion(sign: (serverNonce: string) => string | Promise<string>): Extension {
    return new Extension('entra_assertion', sign);
}

export { entraReqCnf, generateEntraPopKey, signEntraAssertion } from './entra';
export type { EntraAssertionParams } from './entra';

/**
 * Receives the remote desktop's audio through the callback described in README.md, Audio playback.
 * Wave messages contain interleaved 16-bit little-endian PCM in a Uint8Array.
 * Volume messages contain normalized logarithmic protocol levels, not linear amplitude gains.
 * Close messages also precede initial format negotiation and renegotiation; later waves can follow.
 */
export function audioPlayback(callback: (message: unknown) => void): Extension {
    return new Extension('audio_playback', callback);
}

/**
 * Redirects a camera through the callback described in README.md, Camera.
 * Start messages name the size and the format the server picked, `'mjpeg'` or `'h264'`; a stop message ends the capture.
 * The page then pushes one frame at a time with `cameraSample`.
 */
export function camera(callback: (message: unknown) => void): Extension {
    return new Extension('camera', callback);
}

/**
 * Also offer H.264 at every size, ahead of MJPEG, when the page can encode it.
 * The start message's `format` says which the server picked.
 */
export function cameraH264(enable: boolean): Extension {
    return new Extension('camera_h264', enable);
}

/** The camera's name on the remote desktop, in place of `Browser camera`. An empty name is ignored. */
export function cameraName(name: string): Extension {
    return new Extension('camera_name', name);
}

/**
 * One frame for `invokeExtension`, a JPEG or an H.264 access unit as the last start message said.
 * `invokeExtension` returns true when a frame went out and none is left held. It returns false when a frame is held
 * for the next request, when no stream is running (the frame is dropped), or when the server refused the frame and
 * got a Sample Error Response instead.
 * An MJPEG frame replaces any held frame; an H.264 frame queues behind them, since each depends on the ones before it.
 * Push H.264 access units in decode order, as the encoder emits them: the queue keeps that order.
 * When 30 frames are held and none is requested, the queue is dropped and the callback gets a stop and a new start.
 */
export function cameraSample(frame: Uint8Array): Extension {
    return new Extension('camera_sample', frame);
}

/**
 * For `invokeExtension`: how many frames to encode now, the pending requests no held frame answers plus one kept ready.
 * Ask before encoding an H.264 frame, because a dropped one spoils the picture until the next key frame.
 */
export function cameraWanted(): Extension {
    return new Extension('camera_wanted', null);
}

export function kdcProxyUrl(url: string): Extension {
    return new Extension('kdc_proxy_url', url);
}

export function outboundMessageSizeLimit(limit: number): Extension {
    return new Extension('outbound_message_size_limit', limit);
}

export function enableCredssp(enable: boolean): Extension {
    return new Extension('enable_credssp', enable);
}

/**
 * Allow unencrypted Standard RDP Security over a trusted RDCleanPath transport.
 *
 * Enabling this disables TLS and CredSSP protocol negotiation. Use it only when
 * the WebSocket and proxy-to-server transports already provide the required
 * confidentiality and peer authentication.
 */
export function enableStandardRdpSecurity(enable: boolean): Extension {
    return new Extension('enable_standard_rdp_security', enable);
}

export function enableServerPointer(enable: boolean): Extension {
    return new Extension('enable_server_pointer', enable);
}

export function legacyGraphics(enable: boolean): Extension {
    return new Extension('legacy_graphics', enable);
}

/**
 * The `performanceFlags` of the Extended Info Packet (MS-RDPBCGR 2.2.1.11.1.1.1), such as
 * 0x1 to leave out the wallpaper or 0x20 to leave out the cursor shadow on a slow link.
 * Without it the client sends DISABLE_FULLWINDOWDRAG, DISABLE_MENUANIMATIONS and ENABLE_FONT_SMOOTHING.
 */
export function performanceFlags(flags: number): Extension {
    return new Extension('performance_flags', flags);
}

/**
 * Ask the server to scale its user interface, in percent (100 to 500), as Windows'
 * display scale does on a high-density screen.
 * Pair it with a desktop size in device pixels.
 */
export function desktopScaleFactor(percent: number): Extension {
    return new Extension('desktop_scale_factor', percent);
}

/**
 * The session's color depth in bits per pixel.
 * Without it the client asks for 16, or 32 with `legacyGraphics`.
 */
export function colorDepth(bitsPerPixel: 15 | 16 | 24 | 32): Extension {
    return new Extension('color_depth', bitsPerPixel);
}

/** One monitor in desktop coordinates; `right` and `bottom` are inclusive. */
export interface MonitorRect {
    left: number;
    top: number;
    right: number;
    bottom: number;
    primary: boolean;
}

/**
 * Request a session spanning up to 16 monitors (MS-RDPBCGR 2.2.1.3.6).
 * Exactly one monitor must be primary at (0, 0), and the union must be between 200 and 32,766 pixels in each dimension.
 * A valid layout sets the requested desktop size to the monitor union, overriding any configured desktop size.
 * Invalid layouts are logged and ignored.
 */
export function monitorLayout(monitors: MonitorRect[]): Extension {
    return new Extension(
        'monitor_layout',
        monitors.flatMap((m) => [m.left, m.top, m.right, m.bottom, m.primary ? 1 : 0]),
    );
}

/** Reports the server's layout at startup and when it changes, exposing only the primary flag. */
export function monitorLayoutCallback(callback: (monitors: MonitorRect[]) => void): Extension {
    const unpack = (values: number[]) => {
        const monitors: MonitorRect[] = [];
        for (let i = 0; i + 4 < values.length; i += 5) {
            monitors.push({
                left: values[i],
                top: values[i + 1],
                right: values[i + 2],
                bottom: values[i + 3],
                primary: (values[i + 4] & 1) !== 0,
            });
        }
        callback(monitors);
    };
    return new Extension('monitor_layout_callback', unpack);
}

/**
 * The client name the server sees, in place of `ironrdp-web`: Windows shows it as the session's client name.
 * The protocol keeps the first 15 UTF-16 code units, so an emoji takes two. An empty name is ignored.
 */
export function clientName(name: string): Extension {
    return new Extension('client_name', name);
}

/**
 * The keyboard layout to ask for, as a Windows input locale identifier such as 0x409 (US) or 0x809 (UK).
 * Without it, or with 0, the server uses its default layout.
 */
export function keyboardLayout(layoutId: number): Extension {
    return new Extension('keyboard_layout', layoutId);
}

// --- Session statistics ---

/** Asks the session for its figures so far, through `invokeExtension`: see README.md, Session statistics. */
export function stats(): Extension {
    return new Extension('stats', null);
}

// --- RDCleanPath ---

export { RDCleanPathErrorCode } from './RDCleanPathErrorCode';

// --- File transfer (RDP-specific) ---

export { RdpFileTransferProvider } from './RdpFileTransferProvider';
export type {
    RdpFileTransferProviderOptions,
    TransferProgress,
    FileTransferError,
    DownloadHandle,
    UploadHandle,
    DroppedFile,
} from './RdpFileTransferProvider';
export type { FileInfo, FileContentsRequest, FileContentsResponse } from './FileTransfer';
export { FileContentsFlags } from './FileContentsFlags';

// --- Storage backends ---
// Re-export for consumers who want to configure the storageBackend
// option on RdpFileTransferProviderOptions, implement a custom backend,
// or construct a specific backend instance directly.
export type { FileStorageBackend, FileWriteHandle, StorageBackendPreference } from './storage';
export { BlobStorageBackend } from './storage';
export { OpfsStorageBackend } from './storage';
export { detectStorageBackend } from './storage';

// Re-export extension factories for advanced consumers who want to
// register callbacks or invoke file transfer operations directly.
export {
    filesAvailableCallback,
    fileContentsRequestCallback,
    fileContentsResponseCallback,
    lockCallback,
    unlockCallback,
    locksExpiredCallback,
    requestFileContents,
    submitFileContents,
    initiateFileCopy,
    printJobStreamCallbacks,
    PrinterDriverName,
    printerName,
    printerDeviceId,
    printerDriverName,
} from './extensions';
export type { PrintJobStreamCallbacks } from './extensions';
