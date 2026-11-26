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

export function formatListResponseCallback(cb: (ok: boolean) => void): Extension {
    return new Extension('format_list_response_callback', cb as unknown);
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
//
// `printers` announces several printers instead (for example every printer of
// the local computer, each with the driver that the host has for it).
// `onJobStart` then tells which printer a job was sent to, and
// `onPrinterAnnounced` reports whether the host accepted each printer.

export const PrinterDriverName = {
    PostScript: 'MS Publisher Imagesetter',
    MicrosoftPrintToPdf: 'Microsoft Print to PDF',
} as const;

export interface PrintJobStreamCallbacks {
    /** `deviceId` is the printer the job was sent to. */
    onJobStart?: (fileId: number, deviceId: number) => void;
    onJobData: (fileId: number, chunk: Uint8Array) => void;
    onJobComplete: (fileId: number) => void;
    onJobError?: (fileId: number) => void;
    /** The host's answer to a printer announcement; `resultCode` is an NTSTATUS. */
    onPrinterAnnounced?: (deviceId: number, accepted: boolean, resultCode: number) => void;
}

/** A printer announced with {@link printers}. */
export interface RedirectedPrinter {
    /** Queue name shown on the host (usually with "(redirected N)" added). */
    name: string;
    /** Driver the host uses for the queue; defaults to `printerDriverName`, else the platform default. */
    driverName?: string;
    /** RDPDR device ID (a non-zero integer); the lowest free ID from 2 up when omitted. */
    deviceId?: number;
    /** At most one printer is the default; the first one when none is marked. */
    isDefault?: boolean;
    /** Whether the local queue is a network printer (default `true`). */
    network?: boolean;
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

/**
 * Announces these printers instead of the single printer configured above. The
 * list must not be empty, and names and device IDs must be unique.
 */
export function printers(list: RedirectedPrinter[]): Extension {
    return new Extension('printers', list as unknown);
}

// Drive redirection (RDPDR)
//
// `drives` announces folders of the local computer as drives on the host.
// The browser cannot open local files, so every filesystem request of the
// host goes to `driveRequestCallback`; answer each one that has a nonzero
// `id` with `Session.invokeExtension(driveReply({...}))`. Requests may be
// answered in any order but must be carried out in the order they arrive: a
// `close` with `id` 0 (a handle that changed nothing) takes no reply. Directory
// listings are fetched whole (`list`) and enumerated from memory, and the `stat`
// of a `create` reply answers metadata queries until the drive changes. Times are Unix milliseconds; statuses are NTSTATUS
// values (0 is success).

/** A folder announced with {@link drives}. */
export interface RedirectedDrive {
    /** RDPDR device ID, unique among drives and printers. */
    deviceId: number;
    /** Name shown on the host, as in "<name> on <client name>". */
    name: string;
}

/** File metadata in a {@link DriveReply}. */
export interface DriveFileInfo {
    /** Entry name (`list` replies only). */
    name?: string;
    dir: boolean;
    size: number;
    created: number;
    accessed: number;
    written: number;
    changed: number;
    /** FILE_ATTRIBUTE_* bits (read-only 0x1, hidden 0x2, ...). */
    attributes?: number;
    deletePending?: boolean;
}

export type DriveRequest = { id: number; deviceId: number; fileId: number } & (
    | {
          op: 'create';
          /** `\`-separated, relative to the drive root (empty or `\` is the root). */
          path: string;
          desiredAccess: number;
          /** FILE_SUPERSEDE 0, OPEN 1, CREATE 2, OPEN_IF 3, OVERWRITE 4, OVERWRITE_IF 5. */
          disposition: number;
          /** FILE_DIRECTORY_FILE 0x1, FILE_NON_DIRECTORY_FILE 0x40, FILE_DELETE_ON_CLOSE 0x1000, ... */
          options: number;
          attributes: number;
          shareAccess: number;
      }
    | { op: 'close' | 'stat' | 'volume' | 'reset' }
    | { op: 'read'; offset: number; length: number }
    | { op: 'write'; offset: number; data: Uint8Array }
    /** `path` is `\dir\pattern`, where the pattern may hold `*` and `?`. */
    | { op: 'list'; path: string }
    | { op: 'setBasic'; created: number; accessed: number; written: number; changed: number; attributes: number }
    | { op: 'setSize'; size: number }
    | { op: 'setDelete'; delete: boolean }
    | { op: 'rename'; to: string; replace: boolean }
    | { op: 'announced'; accepted: boolean; resultCode: number }
);

/** The answer to a {@link DriveRequest}. */
export interface DriveReply {
    id: number;
    /** NTSTATUS; 0 is success. */
    status: number;
    /** `create`: the new handle. */
    fileId?: number;
    /** `create` (optional) and `stat`: the file's metadata. */
    /** `create`: FILE_SUPERSEDED 0, OPENED 1, CREATED 2, OVERWRITTEN 3. */
    information?: number;
    /** `read`: the bytes read. */
    data?: Uint8Array;
    /** `write`: bytes written. */
    length?: number;
    stat?: DriveFileInfo;
    /** `list`: matching entries, including `.` and `..` when they match. */
    entries?: DriveFileInfo[];
    /** `volume`: total and free bytes. */
    total?: number;
    free?: number;
}

export function drives(list: RedirectedDrive[]): Extension {
    return new Extension('drives', list as unknown);
}

export function driveRequestCallback(cb: (request: DriveRequest) => void): Extension {
    return new Extension('drive_request_callback', cb as unknown);
}

export function driveReply(reply: DriveReply): Extension {
    return new Extension('drive_reply', reply as unknown);
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
