/**
 * RDP-specific extension factories for file transfer.
 *
 * These create Extension objects that are dispatched through the WASM
 * invoke_extension() / extension() mechanism in ironrdp-web.
 */

import { Extension } from '../../../crates/ironrdp-web/pkg/ironrdp_web';
import type { FileInfo } from './FileTransfer';

// Builder-time callback extensions (registered via SessionBuilder.extension())

export function filesAvailableCallback(cb: (files: FileInfo[], clipDataId?: number) => void): Extension {
    return new Extension('files_available_callback', cb as unknown);
}

export function fileContentsRequestCallback(
    cb: (request: {
        streamId: number;
        index: number;
        flags: number;
        position: number;
        size: number;
        dataId?: number;
    }) => void,
): Extension {
    return new Extension('file_contents_request_callback', cb as unknown);
}

export function fileContentsResponseCallback(
    cb: (response: { streamId: number; isError: boolean; data: Uint8Array }) => void,
): Extension {
    return new Extension('file_contents_response_callback', cb as unknown);
}

export function lockCallback(cb: (dataId: number) => void): Extension {
    return new Extension('lock_callback', cb as unknown);
}

export function unlockCallback(cb: (dataId: number) => void): Extension {
    return new Extension('unlock_callback', cb as unknown);
}

export function locksExpiredCallback(cb: (clipDataIds: Uint32Array) => void): Extension {
    return new Extension('locks_expired_callback', cb as unknown);
}

// Virtual printer (RDPDR) extensions
//
// Registering `printJobStreamCallbacks` activates the browser-side virtual
// printer device. The printer backend streams write chunks as they arrive
// instead of buffering a completed job in WASM memory. By default, the web
// connector follows FreeRDP's macOS heuristic where possible:
// browser-reported macOS 14+ uses Microsoft Print to PDF, and other clients use
// MS Publisher Imagesetter for PostScript bytes. Use `printerDriverName` when
// the target host needs a different installed printer driver. Jobs larger than
// 128 MiB are rejected, and queued write chunks are bounded to protect browser
// memory. `printerName`, `printerDeviceId`, and `printerDriverName` are
// optional; sensible defaults are applied when omitted.

export const PrinterDriverName = {
    PostScript: 'MS Publisher Imagesetter',
    MicrosoftPrintToPdf: 'Microsoft Print to PDF',
} as const;

export interface PrintJobStreamCallbacks {
    onJobStart?: (fileId: number) => void;
    onJobData: (fileId: number, chunk: Uint8Array) => void;
    onJobComplete: (fileId: number) => void;
    onJobError?: (fileId: number) => void;
}

export function printJobStreamCallbacks(callbacks: PrintJobStreamCallbacks): Extension {
    return new Extension('print_job_stream_callbacks', callbacks as unknown);
}

export function printerName(name: string): Extension {
    return new Extension('printer_name', name);
}

export function printerDeviceId(id: number): Extension {
    return new Extension('printer_device_id', id);
}

export function printerDriverName(driverName: string): Extension {
    return new Extension('printer_driver_name', driverName);
}

// Runtime operation extensions (invoked via Session.invokeExtension())

export function requestFileContents(params: {
    stream_id: number;
    file_index: number;
    flags: number;
    position: number;
    size: number;
    clip_data_id?: number;
}): Extension {
    return new Extension('request_file_contents', params as unknown);
}

export function submitFileContents(params: { stream_id: number; is_error: boolean; data: Uint8Array }): Extension {
    return new Extension('submit_file_contents', params as unknown);
}

export function initiateFileCopy(files: FileInfo[]): Extension {
    return new Extension('initiate_file_copy', files as unknown);
}
