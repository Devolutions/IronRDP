import type { InputTransaction } from './InputTransaction';
import type { DesktopSize } from './DesktopSize';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardData } from './ClipboardData';
import type { FileInfo } from './FileTransfer';

export interface Session {
    run(): Promise<SessionTerminationInfo>;
    desktopSize(): DesktopSize;
    applyInputs(transaction: InputTransaction): void;
    releaseAllInputs(): void;
    synchronizeLockKeys(scrollLock: boolean, numLock: boolean, capsLock: boolean, kanaLock: boolean): void;
    invokeExtension(value: unknown): unknown;
    shutdown(): void;
    onClipboardPaste(data: ClipboardData): Promise<void>;
    resize(
        width: number,
        height: number,
        scaleFactor?: number | null,
        physicalWidth?: number | null,
        physicalHeight?: number | null,
    ): void;
    supportsUnicodeKeyboardShortcuts(): boolean;

    // File transfer methods

    /**
     * Request file contents from remote (download).
     *
     * Per MS-RDPECLIP 2.2.5.3.1, sends FileContentsRequest PDU to remote.
     * Response arrives via fileContentsResponseCallback.
     *
     * @param streamId - Unique stream identifier for this transfer
     * @param fileIndex - Index in file list (0-based)
     * @param flags - FileContentsFlags: 0x1 (SIZE) or 0x2 (RANGE)
     * @param position - Byte offset for RANGE requests
     * @param size - Number of bytes requested for RANGE requests
     * @param clipDataId - Optional lock ID from lockClipboard()
     *
     * @example
     * ```typescript
     * // Request file size
     * session.requestFileContents(1, 0, 0x1, 0, 8, clipDataId);
     * // Request file data chunk
     * session.requestFileContents(1, 0, 0x2, 0, 65536, clipDataId);
     * ```
     */
    requestFileContents(
        streamId: number,
        fileIndex: number,
        flags: number,
        position: number,
        size: number,
        clipDataId?: number,
    ): void;

    /**
     * Submit file contents to remote (upload response).
     *
     * Per MS-RDPECLIP 2.2.5.3.2, sends FileContentsResponse PDU in response
     * to remote's FileContentsRequest (received via fileContentsRequestCallback).
     *
     * @param streamId - Stream ID from the request
     * @param isError - True to indicate error (data unavailable/access denied)
     * @param data - File contents: 8-byte LE u64 for SIZE, byte range for DATA
     *
     * @example
     * ```typescript
     * // Respond with file size
     * const sizeBytes = new Uint8Array(8);
     * new DataView(sizeBytes.buffer).setBigUint64(0, BigInt(file.size), true);
     * session.submitFileContents(streamId, false, sizeBytes);
     *
     * // Respond with file data chunk
     * const chunk = await file.slice(offset, offset + size).arrayBuffer();
     * session.submitFileContents(streamId, false, new Uint8Array(chunk));
     * ```
     */
    submitFileContents(streamId: number, isError: boolean, data: Uint8Array): void;

    /**
     * Initiate file copy operation (upload to remote).
     *
     * Per MS-RDPECLIP 2.2.5.2, sends FormatList with FileGroupDescriptorW
     * containing file metadata. Remote will request file contents via
     * FileContentsRequest PDUs, which arrive via fileContentsRequestCallback.
     *
     * @param files - Array of file metadata to advertise
     *
     * @example
     * ```typescript
     * const files: FileInfo[] = [{
     *   name: 'document.txt',
     *   size: 1024,
     *   lastModified: Date.now(), // JS timestamp (ms since Unix epoch)
     * }];
     * session.initiateFileCopy(files);
     * ```
     */
    initiateFileCopy(files: FileInfo[]): void;
}
