import { Extension } from '../../../crates/ironrdp-web/pkg/ironrdp_web';
import type { FileInfo, FileContentsRequest, FileContentsResponse } from './FileTransfer';
import { FileContentsFlags } from './FileContentsFlags';
import {
    filesAvailableCallback,
    fileContentsRequestCallback,
    fileContentsResponseCallback,
    lockCallback,
    unlockCallback,
    locksExpiredCallback,
    requestFileContents,
    submitFileContents,
    initiateFileCopy,
} from './extensions';

/**
 * Minimal session interface for extension-based file transfer.
 * Protocol-agnostic - only requires invokeExtension().
 */
interface ExtensionSession {
    invokeExtension(ext: Extension): unknown;
}

/**
 * Configuration options for RdpFileTransferProvider.
 */
export interface RdpFileTransferProviderOptions {
    /**
     * Chunk size in bytes for file transfers.
     * Default: 65536 (64KB)
     */
    chunkSize?: number;

    /**
     * Called when an upload begins (before the FormatList is sent to the remote).
     * Use this to suppress clipboard monitoring so the 100ms polling loop does
     * not clobber the file upload's FormatList with a text/image update.
     */
    onUploadStarted?: () => void;

    /**
     * Called when an upload finishes (success, failure, or dispose).
     * Use this to resume clipboard monitoring after {@link onUploadStarted}.
     */
    onUploadFinished?: () => void;
}

/**
 * Progress information for file transfer operations.
 */
export interface TransferProgress {
    /** Unique identifier for this transfer operation, stable across the lifetime of the transfer */
    transferId: number;
    /** File index in the transfer list */
    fileIndex: number;
    /** File name */
    fileName: string;
    /** Number of bytes transferred so far */
    bytesTransferred: number;
    /** Total file size in bytes */
    totalBytes: number;
    /** Transfer progress as percentage (0-100) */
    percentage: number;
}

/**
 * Error information for file transfer operations.
 */
export interface FileTransferError {
    /** Error message */
    message: string;
    /** Unique transfer identifier (if applicable) */
    transferId?: number;
    /** File index that failed (if applicable) */
    fileIndex?: number;
    /** File name that failed (if applicable) */
    fileName?: string;
    /** Transfer direction that caused the error (if applicable) */
    direction?: 'download' | 'upload';
    /** Underlying error cause */
    cause?: unknown;
}

/**
 * Result of initiating a file download via {@link RdpFileTransferProvider.downloadFile}.
 */
export interface DownloadHandle {
    /** Unique identifier for this download, available synchronously before the download completes */
    transferId: number;
    /** Promise that resolves with the downloaded file blob */
    completion: Promise<Blob>;
}

/**
 * A file extracted from a drag-and-drop event, with optional path metadata
 * from directory traversal via the File and Directory Entries API.
 *
 * When a folder is dropped, its contents are recursively enumerated and each
 * file is returned with a {@link path} relative to the drop root.  Directory
 * entries themselves are included with {@link isDirectory} set to `true` and a
 * `null` {@link file} handle (they carry no data, only structure).
 */
export interface DroppedFile {
    /** Browser File handle.  `null` for directory-only entries. */
    file: File | null;
    /** File or directory basename (e.g. `"report.pdf"` or `"images"`). */
    name: string;
    /** Size in bytes.  Always 0 for directory entries. */
    size: number;
    /** Last-modified timestamp (ms since Unix epoch, same as `File.lastModified`). */
    lastModified: number;
    /**
     * Relative directory path within the dropped collection, using `\` as
     * separator to match the Windows wire convention (MS-RDPECLIP 3.1.1.2).
     * `undefined` for entries at the drop root.
     *
     * @example
     * // File "photo.png" inside dropped folder "docs/images":
     * { name: "photo.png", path: "docs\\images", ... }
     */
    path?: string;
    /** Whether this entry represents a directory rather than a file. */
    isDirectory?: boolean;
}

/**
 * Result of initiating a file upload via {@link RdpFileTransferProvider.uploadFiles}.
 */
export interface UploadHandle {
    /** Map of file index to unique transfer identifier for each file in the batch */
    transferIds: Map<number, number>;
    /** Promise that resolves when all files in the batch have been uploaded */
    completion: Promise<void>;
}

/**
 * Internal state for tracking active file transfers.
 */
interface TransferState {
    fileInfo: FileInfo;
    fileIndex: number;
    streamId: number;
    clipDataId?: number;
    expectedSize?: number;
    chunks: Uint8Array[];
    bytesReceived: number;
    resolve: (blob: Blob) => void;
    reject: (error: Error) => void;
}

/**
 * Internal state for tracking file uploads.
 */
interface UploadState {
    /** File handles indexed by position in the file list.  `null` entries
     *  represent directory-only entries which carry no data. */
    files: (File | null)[];
    /** DroppedFile metadata for each entry, parallel to `files`. */
    droppedFiles: DroppedFile[];
    /** File indices that have permanently failed (e.g. read timeout). */
    failedFiles: Set<number>;
    expectedFileCount: number;
    completedFiles: Set<number>;
    /** Tracks total bytes served per file index across all RANGE responses.
     *  Used for robust upload completion detection regardless of chunk order. */
    bytesServed: Map<number, number>;
    activeReaders: Map<number, FileReader>;
    readerTimeouts: Map<number, ReturnType<typeof setTimeout>>;
    /** Maps file index to unique transfer identifier for each file in the batch. */
    transferIds: Map<number, number>;
    resolve: () => void;
    reject: (error: Error) => void;
    /** True when this state was rebuilt from retainedFiles for a re-paste.
     *  Skips onUploadStarted/onUploadFinished lifecycle callbacks since the
     *  original upload already completed and monitoring is in the right state. */
    isRePaste?: boolean;
}

type EventHandler<T extends unknown[]> = (...args: T) => void;

type EventMap = {
    'download-progress': [TransferProgress];
    'upload-progress': [TransferProgress];
    'download-complete': [FileInfo, Blob, number, number];
    'upload-complete': [File, number, number];
    /** Emitted when an upload batch begins (both initial paste and re-paste).
     *  Provides the full batch of transferIds and DroppedFile metadata so
     *  listeners can register all transfers eagerly before progress events arrive. */
    'upload-batch-started': [Map<number, number>, DroppedFile[]];
    'files-available': [FileInfo[]];
    error: [FileTransferError];
};

/**
 * RdpFileTransferProvider provides a high-level API for bidirectional file transfer
 * in browser-based RDP sessions.
 *
 * This class wraps the low-level WASM file transfer API and handles:
 * - State management for downloads and uploads
 * - Chunking and reassembly
 * - Progress tracking
 * - Browser integration helpers (file picker, drag-and-drop)
 *
 * ## Clipboard Locking
 *
 * Clipboard locks are managed automatically by the Rust cliprdr processor. When
 * the remote copies files (FormatList containing FileGroupDescriptorW), a lock
 * is acquired automatically. The lock ID is passed to RdpFileTransferProvider via
 * the filesAvailable callback and used in all subsequent FileContentsRequest
 * PDUs. Lock lifecycle (expiry on clipboard change, Unlock PDU emission) is
 * handled entirely by the Rust layer - no explicit lock/unlock calls are needed.
 *
 * ## Error Handling Best Practices
 *
 * Production applications should implement comprehensive error handling:
 *
 * @example Basic Usage
 * ```typescript
 * const provider = new RdpFileTransferProvider({ chunkSize: 64 * 1024 });
 * component.enableFileTransfer(provider);
 * await component.connect(config);
 *
 * // Handle downloads from remote
 * provider.on('files-available', async (files) => {
 *   for (const file of files) {
 *     const { completion } = provider.downloadFile(file, files.indexOf(file));
 *     const blob = await completion;
 *     saveAs(blob, file.name);
 *   }
 * });
 *
 * // Handle uploads using file picker
 * const files = await provider.showFilePicker({ multiple: true });
 * provider.uploadFiles(files);
 * ```
 *
 * @example Error Handling
 * ```typescript
 * provider.on('error', (error) => {
 *   console.error('Transfer error:', error.message);
 *   if (error.fileName) {
 *     showNotification(`Failed to transfer ${error.fileName}: ${error.message}`);
 *   }
 * });
 * ```
 *
 * @example Handling Lock Expiration
 * ```typescript
 * provider.on('error', (error) => {
 *   if (error.message.includes('lock expired')) {
 *     showNotification(
 *       'File download timed out',
 *       'The transfer took too long. Try a faster connection or smaller files.',
 *       'warning'
 *     );
 *   }
 * });
 * ```
 */
export class RdpFileTransferProvider {
    /** Maximum file size for downloads (2GB) to prevent browser out-of-memory errors */
    private static readonly MAX_FILE_SIZE = 2 * 1024 * 1024 * 1024;
    /** Timeout for FileReader operations (60 seconds) to prevent stalled uploads */
    private static readonly FILE_READER_TIMEOUT_MS = 60 * 1000;
    /** Maximum recursion depth when traversing dropped directories. */
    private static readonly MAX_DIRECTORY_DEPTH = 32;
    /** Maximum total entries (files + directories) collected from a single drop. */
    private static readonly MAX_DIRECTORY_ENTRIES = 1000;

    private session?: ExtensionSession;
    private readonly chunkSize: number;
    private readonly onUploadStarted?: () => void;
    private readonly onUploadFinished?: () => void;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    private readonly eventHandlers: Map<keyof EventMap, Set<EventHandler<any>>> = new Map();

    private activeDownloads: Map<number, TransferState> = new Map();
    private uploadState?: UploadState;
    // DroppedFile metadata retained after upload completes so re-paste works
    // without re-dropping. Cleared when a new upload starts or the manager is disposed.
    private retainedFiles?: DroppedFile[];
    private availableFiles: FileInfo[] = [];
    // Clipboard lock ID received with the most recent file list. The Rust layer
    // acquires this lock automatically when FileGroupDescriptorW is detected in
    // the FormatList. Downloads use it instead of explicit lock/unlock calls -
    // the lock lifecycle is managed entirely by the Rust cliprdr processor.
    private clipDataId?: number;
    private nextStreamId: number = 1;
    private disposed: boolean = false;

    constructor(options?: RdpFileTransferProviderOptions) {
        this.chunkSize = options?.chunkSize ?? 65536; // Default: 64KB
        this.onUploadStarted = options?.onUploadStarted;
        this.onUploadFinished = options?.onUploadFinished;
    }

    /**
     * Set the session instance after connection is established.
     * Called by the web component's connect() flow via the FileTransferProvider interface.
     */
    setSession(session: ExtensionSession): void {
        this.session = session;
    }

    private ensureSession(): ExtensionSession {
        if (this.session === undefined) {
            throw new Error('RdpFileTransferProvider: Session not available. Ensure connect() has been called.');
        }
        return this.session;
    }

    // --- Extension-based session method wrappers ---
    // These replace direct session.requestFileContents() etc. calls
    // with invokeExtension() to keep the Session interface protocol-agnostic.

    private sendRequestFileContents(
        streamId: number,
        fileIndex: number,
        flags: number,
        position: number,
        size: number,
        clipDataId?: number,
    ): void {
        this.ensureSession().invokeExtension(
            requestFileContents({
                stream_id: streamId,
                file_index: fileIndex,
                flags,
                position,
                size,
                clip_data_id: clipDataId,
            }),
        );
    }

    private sendSubmitFileContents(streamId: number, isError: boolean, data: Uint8Array): void {
        this.session?.invokeExtension(submitFileContents({ stream_id: streamId, is_error: isError, data }));
    }

    private sendInitiateFileCopy(files: FileInfo[]): void {
        this.ensureSession().invokeExtension(initiateFileCopy(files));
    }

    /**
     * Returns the extension objects to register on the SessionBuilder before connect().
     * Implements the FileTransferProvider interface.
     */
    getBuilderExtensions(): Extension[] {
        return [
            filesAvailableCallback((files: FileInfo[], clipDataId?: number) =>
                this.handleFilesAvailable(files, clipDataId),
            ),
            fileContentsRequestCallback((req: FileContentsRequest) => this.handleFileContentsRequest(req)),
            fileContentsResponseCallback((resp: FileContentsResponse) => this.handleFileContentsResponse(resp)),
            lockCallback((id: number) => this.handleLock(id)),
            unlockCallback((id: number) => this.handleUnlock(id)),
            locksExpiredCallback((ids: Uint32Array) => this.handleLocksExpired(ids)),
        ];
    }

    /**
     * Register an event handler.
     *
     * @param event - Event name
     * @param handler - Event handler function
     *
     * @example
     * ```typescript
     * manager.on('download-progress', (progress) => {
     *   console.log(`${progress.fileName}: ${progress.percentage}%`);
     * });
     * ```
     */
    on<K extends keyof EventMap>(event: K, handler: EventHandler<EventMap[K]>): void {
        if (!this.eventHandlers.has(event)) {
            this.eventHandlers.set(event, new Set());
        }
        this.eventHandlers.get(event)!.add(handler);
    }

    /**
     * Unregister an event handler.
     *
     * @param event - Event name
     * @param handler - Event handler function to remove
     */
    off<K extends keyof EventMap>(event: K, handler: EventHandler<EventMap[K]>): void {
        const handlers = this.eventHandlers.get(event);
        if (handlers) {
            handlers.delete(handler);
        }
    }

    /**
     * Emit an event to all registered handlers.
     */
    private emit<K extends keyof EventMap>(event: K, ...args: EventMap[K]): void {
        const handlers = this.eventHandlers.get(event);
        if (handlers) {
            for (const handler of handlers) {
                try {
                    handler(...args);
                } catch (error) {
                    console.error(`Error in ${event} handler:`, error);
                }
            }
        }
    }

    /**
     * Download a single file from the remote.
     *
     * Returns a {@link DownloadHandle} with a `transferId` available synchronously
     * (for immediate UI association) and a `completion` promise that resolves with
     * the downloaded blob.
     *
     * @param fileInfo - File metadata from 'files-available' event
     * @param fileIndex - Index of the file in the original file list
     * @returns Handle with synchronous transferId and async completion
     *
     * @example
     * ```typescript
     * const { transferId, completion } = manager.downloadFile(fileInfo, 0);
     * // transferId is available immediately for UI binding
     * const blob = await completion;
     * saveAs(blob, fileInfo.name);
     * ```
     */
    downloadFile(fileInfo: FileInfo, fileIndex: number): DownloadHandle {
        // Generate unique stream ID (serves as transferId)
        const streamId = this.generateStreamId();

        const completion = this.executeDownload(fileInfo, fileIndex, streamId);

        return { transferId: streamId, completion };
    }

    /**
     * Internal: execute the async download workflow for a single file.
     */
    private async executeDownload(fileInfo: FileInfo, fileIndex: number, streamId: number): Promise<Blob> {
        // Use the clipboard lock acquired by the Rust layer when the file list
        // was received. The lock lifecycle (creation, expiry, Unlock PDUs) is
        // managed entirely by the cliprdr processor - we just pass the ID
        // through to FileContentsRequest so the server associates requests with
        // the correct clipboard snapshot.
        const clipDataId = this.clipDataId;

        // Create transfer state
        const transferPromise = new Promise<Blob>((resolve, reject) => {
            const state: TransferState = {
                fileInfo,
                fileIndex,
                streamId,
                clipDataId,
                chunks: [],
                bytesReceived: 0,
                resolve,
                reject,
            };

            this.activeDownloads.set(streamId, state);
        });

        // Request file size first (flags = 0x1).
        // Per MS-RDPECLIP 2.2.5.3, SIZE requests MUST set cbRequested to 8.
        try {
            this.sendRequestFileContents(streamId, fileIndex, FileContentsFlags.SIZE, 0, 8, clipDataId);
        } catch (error) {
            this.activeDownloads.delete(streamId);
            const err: FileTransferError = {
                message: 'Failed to request file size',
                transferId: streamId,
                fileIndex,
                fileName: fileInfo.name,
                direction: 'download',
                cause: error,
            };
            this.emit('error', err);
            throw new Error(err.message, { cause: error });
        }

        return transferPromise;
    }

    /**
     * Download multiple files sequentially.
     *
     * @param files - Array of FileInfo from 'files-available' event
     * @returns AsyncGenerator yielding file/blob pairs as they complete
     *
     * @example
     * ```typescript
     * for await (const { file, blob } of manager.downloadFiles(files)) {
     *   saveAs(blob, file.name);
     * }
     * ```
     */
    async *downloadFiles(files: FileInfo[]): AsyncGenerator<{ file: FileInfo; blob: Blob; transferId: number }> {
        for (let i = 0; i < files.length; i++) {
            const file = files[i];
            const { transferId, completion } = this.downloadFile(file, i);
            const blob = await completion;
            yield { file, blob, transferId };
        }
    }

    /**
     * Download multiple files concurrently with configurable parallelism.
     *
     * This method initiates multiple file downloads in parallel, improving
     * performance for multi-file transfers. Each file uses an independent
     * clipboard lock and stream ID.
     *
     * @param files - Array of FileInfo from 'files-available' event
     * @param options - Download options
     * @param options.maxConcurrent - Maximum concurrent downloads (default: 3)
     * @returns Promise resolving to map of fileIndex to Blob
     *
     * @example
     * ```typescript
     * const blobs = await manager.downloadFilesConcurrent(files, { maxConcurrent: 5 });
     * files.forEach((file, i) => saveAs(blobs.get(i)!, file.name));
     * ```
     */
    async downloadFilesConcurrent(
        files: FileInfo[],
        options: { maxConcurrent?: number } = {},
    ): Promise<Map<number, Blob>> {
        const maxConcurrent = options.maxConcurrent ?? 3;

        const results = new Map<number, Blob>();
        const errors: Array<{ index: number; error: unknown }> = [];

        // Create download tasks
        const downloadTasks = files.map((file, index) => async () => {
            try {
                const { completion } = this.downloadFile(file, index);
                const blob = await completion;
                results.set(index, blob);
            } catch (error) {
                errors.push({ index, error });
            }
        });

        // Execute with concurrency limit.
        // Each task promise catches its own errors so that Promise.race/Promise.all
        // never reject — errors are collected in the `errors` array above.
        const executing: Array<Promise<void>> = [];
        for (const task of downloadTasks) {
            const promise = task().finally(() => {
                executing.splice(executing.indexOf(promise), 1);
            });

            executing.push(promise);

            if (executing.length >= maxConcurrent) {
                await Promise.race(executing);
            }
        }

        // Wait for remaining downloads (all rejections already caught)
        await Promise.all(executing);

        // Throw if any downloads failed
        if (errors.length > 0) {
            throw new Error(
                `Failed to download ${errors.length} file(s): ` +
                    errors.map((e) => `file ${e.index}: ${e.error}`).join(', '),
            );
        }

        return results;
    }

    /**
     * Upload files to the remote.
     *
     * Returns an {@link UploadHandle} with per-file `transferIds` available
     * synchronously (for immediate UI association) and a `completion` promise
     * that resolves when all files have been uploaded.
     *
     * @param files - Array of File objects to upload
     * @returns Handle with synchronous transferIds and async completion
     *
     * @example
     * ```typescript
     * const files = await manager.showFilePicker({ multiple: true });
     * const { transferIds, completion } = manager.uploadFiles(files);
     * // transferIds available immediately for UI binding
     * await completion;
     * ```
     */
    uploadFiles(files: File[] | DroppedFile[]): UploadHandle {
        if (this.uploadState !== undefined) {
            throw new Error('Upload already in progress');
        }

        // New upload supersedes any retained files from a previous batch
        this.retainedFiles = undefined;

        // Normalize: accept both plain File[] (backward compat) and DroppedFile[]
        const dropped: DroppedFile[] = RdpFileTransferProvider.normalizeToDroppedFiles(files);

        // Generate unique transfer IDs for each entry in the batch
        const transferIds = new Map<number, number>();
        for (let i = 0; i < dropped.length; i++) {
            transferIds.set(i, this.generateStreamId());
        }

        // Build FileInfo with path and isDirectory so the WASM layer can
        // encode FileGroupDescriptorW with correct relative paths and
        // FILE_ATTRIBUTE_DIRECTORY attributes.
        const fileInfos: FileInfo[] = dropped.map((d) => ({
            name: d.name,
            size: d.size,
            lastModified: d.lastModified,
            path: d.path,
            isDirectory: d.isDirectory,
        }));

        // Directory entries carry no data - only count actual files toward
        // the expected completion count so progress percentages make sense.
        const fileCount = dropped.filter((d) => d.isDirectory !== true).length;

        // Extract File handles parallel to dropped[], null for directory entries
        const fileHandles: (File | null)[] = dropped.map((d) => d.file);

        // Create completion promise
        const completion = new Promise<void>((resolve, reject) => {
            // Store upload state with completion tracking
            this.uploadState = {
                files: fileHandles,
                droppedFiles: dropped,
                failedFiles: new Set(),
                expectedFileCount: fileCount,
                completedFiles: new Set(),
                bytesServed: new Map(),
                activeReaders: new Map(),
                readerTimeouts: new Map(),
                transferIds,
                resolve,
                reject,
            };

            // Suppress clipboard monitoring briefly so the polling loop does not
            // clobber our FormatList with a text/image update. Resume immediately
            // after the FormatList is sent - the suppression window only needs to
            // cover the race between suppressMonitoring() and the wire send.
            // Upload state tracking continues independently via this.uploadState.
            this.onUploadStarted?.();

            // Initiate file copy (broadcasts file list to remote)
            try {
                this.sendInitiateFileCopy(fileInfos);
                this.emit('upload-batch-started', transferIds, dropped);
            } catch (error) {
                this.uploadState = undefined;
                const err: FileTransferError = {
                    message: 'Failed to initiate file upload',
                    direction: 'upload',
                    cause: error,
                };
                this.emit('error', err);
                reject(new Error(err.message, { cause: error }));
            } finally {
                // Resume monitoring regardless of success/failure. The brief
                // suppression window is intentionally short - just long enough
                // to prevent the clipboard poll from racing with our FormatList.
                this.onUploadFinished?.();
            }
        });

        return { transferIds, completion };
    }

    /**
     * Show a file picker dialog and return selected files.
     *
     * Note: This must be called in response to a user gesture (e.g., button click)
     * due to browser security restrictions.
     *
     * @param options - File picker options
     * @returns Promise resolving to selected File objects
     *
     * @example
     * ```typescript
     * button.onclick = async () => {
     *   const files = await manager.showFilePicker({ multiple: true, accept: 'image/*' });
     *   await manager.uploadFiles(files);
     * };
     * ```
     */
    showFilePicker(options?: { multiple?: boolean; accept?: string }): Promise<File[]> {
        return new Promise<File[]>((resolve) => {
            // Create hidden file input
            const input = document.createElement('input');
            input.type = 'file';
            input.style.display = 'none';

            if (options?.multiple === true) {
                input.multiple = true;
            }
            if (options?.accept !== undefined && options.accept.length > 0) {
                input.accept = options.accept;
            }

            // Handle file selection
            input.addEventListener('change', () => {
                const files = Array.from(input.files || []);
                cleanup();
                window.removeEventListener('focus', onFocus);
                resolve(files);
            });

            // Handle cancellation - the 'cancel' event is supported in modern
            // browsers (Chrome 113+). For older browsers, fall back to detecting
            // window focus after the picker closes without a selection.
            let settled = false;
            const cleanup = () => {
                if (settled) return;
                settled = true;
                if (input.parentNode) {
                    document.body.removeChild(input);
                }
            };

            input.addEventListener('cancel', () => {
                cleanup();
                window.removeEventListener('focus', onFocus);
                resolve([]);
            });

            // Fallback: if the browser doesn't fire 'cancel', resolve empty
            // when the window regains focus after the picker is dismissed.
            const onFocus = () => {
                // 300 ms gives the browser enough time to schedule the
                // 'change' event (macrotask) before we treat focus as a
                // cancellation signal. This path only runs when the picker
                // is dismissed without selecting files, so the latency is
                // invisible to the user.
                setTimeout(() => {
                    if (!settled) {
                        cleanup();
                        resolve([]);
                    }
                    window.removeEventListener('focus', onFocus);
                }, 300);
            };
            window.addEventListener('focus', onFocus);

            // Add to DOM and trigger click
            document.body.appendChild(input);
            input.click();
        });
    }

    /**
     * Extract files (and recursively traverse directories) from a drag-and-drop event.
     *
     * Uses the File and Directory Entries API (`webkitGetAsEntry`) which is
     * supported across all major browsers (Chrome, Firefox, Safari, Edge).
     * Falls back to `getAsFile()` when the Entries API is unavailable.
     *
     * Directory entries are included in the result with `isDirectory: true`
     * and a `null` file handle.  Files inside directories have their
     * {@link DroppedFile.path} set to the relative backslash-separated path.
     *
     * @param event - DragEvent from drop handler
     * @returns Promise resolving to an array of DroppedFile descriptors
     *
     * @example
     * ```typescript
     * dropZone.addEventListener('drop', async (e) => {
     *   const files = await manager.handleDrop(e);
     *   manager.uploadFiles(files);
     * });
     * ```
     */
    async handleDrop(event: DragEvent): Promise<DroppedFile[]> {
        event.preventDefault();

        const results: DroppedFile[] = [];

        if (event.dataTransfer?.items) {
            // Collect FileSystemEntry references synchronously - DataTransferItem
            // references become invalid once the event handler returns.
            const entries: FileSystemEntry[] = [];
            const fallbackFiles: File[] = [];

            for (const item of event.dataTransfer.items) {
                if (item.kind !== 'file') continue;

                const entry = item.webkitGetAsEntry?.();
                if (entry) {
                    entries.push(entry);
                } else {
                    // Browser does not support webkitGetAsEntry - fall back
                    const file = item.getAsFile();
                    if (file) {
                        fallbackFiles.push(file);
                    }
                }
            }

            // Async traversal is safe now that we hold entry references
            for (const entry of entries) {
                await this.traverseEntry(entry, undefined, results, 0);
            }

            // Append any fallback files (flat, no path metadata)
            for (const file of fallbackFiles) {
                results.push({
                    file,
                    name: file.name,
                    size: file.size,
                    lastModified: file.lastModified,
                });
            }
        } else if (event.dataTransfer?.files) {
            // Fallback: no items API, use files list (no directory support)
            for (const file of Array.from(event.dataTransfer.files)) {
                results.push({
                    file,
                    name: file.name,
                    size: file.size,
                    lastModified: file.lastModified,
                });
            }
        }

        return results;
    }

    /**
     * Normalize a plain `File[]` or `DroppedFile[]` into `DroppedFile[]`.
     * Allows `uploadFiles` to accept either type for backward compatibility.
     */
    private static normalizeToDroppedFiles(files: File[] | DroppedFile[]): DroppedFile[] {
        if (files.length === 0) return [];

        // Duck-type check: DroppedFile always has a `file` property (File | null)
        // while a plain File does not.  `isDirectory` is optional so we only
        // check for `file` to distinguish the two shapes.
        const first = files[0];
        if ('file' in first) {
            return files as DroppedFile[];
        }

        // Plain File[] - wrap each one
        return (files as File[]).map((f) => ({
            file: f,
            name: f.name,
            size: f.size,
            lastModified: f.lastModified,
        }));
    }

    /**
     * Recursively traverse a FileSystemEntry, collecting files and directory
     * entries into `results`.
     */
    private async traverseEntry(
        entry: FileSystemEntry,
        parentPath: string | undefined,
        results: DroppedFile[],
        depth: number,
    ): Promise<void> {
        if (results.length >= RdpFileTransferProvider.MAX_DIRECTORY_ENTRIES) return;

        if (depth > RdpFileTransferProvider.MAX_DIRECTORY_DEPTH) {
            console.warn(
                `Skipping "${entry.name}": directory depth exceeds ${RdpFileTransferProvider.MAX_DIRECTORY_DEPTH}`,
            );
            return;
        }

        if (entry.isFile) {
            const file = await new Promise<File>((resolve, reject) => {
                (entry as FileSystemFileEntry).file(resolve, reject);
            });
            results.push({
                file,
                name: file.name,
                size: file.size,
                lastModified: file.lastModified,
                path: parentPath,
            });
        } else if (entry.isDirectory) {
            const dirPath = parentPath !== undefined ? `${parentPath}\\${entry.name}` : entry.name;

            // Include the directory entry itself so the remote sees the folder structure
            results.push({
                file: null,
                name: entry.name,
                size: 0,
                lastModified: 0,
                path: parentPath,
                isDirectory: true,
            });

            const reader = (entry as FileSystemDirectoryEntry).createReader();
            const children = await RdpFileTransferProvider.readAllDirectoryEntries(reader);
            for (const child of children) {
                await this.traverseEntry(child, dirPath, results, depth + 1);
            }
        }
    }

    /**
     * Read all entries from a FileSystemDirectoryReader.  Chromium-based
     * browsers return at most 100 entries per `readEntries()` call, so we
     * must loop until an empty batch is returned.
     */
    private static readAllDirectoryEntries(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
        return new Promise<FileSystemEntry[]>((resolve, reject) => {
            const all: FileSystemEntry[] = [];
            const readBatch = (): void => {
                reader.readEntries((entries) => {
                    if (entries.length === 0) {
                        resolve(all);
                    } else {
                        all.push(...entries);
                        readBatch();
                    }
                }, reject);
            };
            readBatch();
        });
    }

    /**
     * Prevent default drag-over behavior to enable drop target.
     *
     * This must be called in the dragover event handler for drag-and-drop to work.
     *
     * @param event - DragEvent from dragover handler
     *
     * @example
     * ```typescript
     * dropZone.addEventListener('dragover', (e) => manager.handleDragOver(e));
     * ```
     */
    handleDragOver(event: DragEvent): void {
        event.preventDefault();
    }

    /**
     * Cleanup resources and unregister callbacks.
     *
     * Call this when the session is terminating or RdpFileTransferProvider is no longer needed.
     */
    dispose(): void {
        this.disposed = true;

        // Cancel active downloads (lock cleanup is handled by the Rust layer)
        for (const state of this.activeDownloads.values()) {
            state.chunks = [];
            state.reject(new Error('RdpFileTransferProvider disposed'));
        }
        this.activeDownloads.clear();

        // Clean up active FileReaders, clear timeouts, and reject upload promise
        if (this.uploadState !== undefined) {
            for (const timeout of this.uploadState.readerTimeouts.values()) {
                clearTimeout(timeout);
            }
            this.uploadState.readerTimeouts.clear();

            for (const reader of this.uploadState.activeReaders.values()) {
                reader.abort();
            }
            this.uploadState.activeReaders.clear();

            this.uploadState.reject(new Error('RdpFileTransferProvider disposed'));
        }
        this.uploadState = undefined;
        this.retainedFiles = undefined;

        // Clear available files and lock reference
        this.availableFiles = [];
        this.clipDataId = undefined;

        // Clear event handlers
        this.eventHandlers.clear();
    }

    // ==================== Callback Handlers ====================

    private handleFilesAvailable(files: FileInfo[], clipDataId?: number): void {
        // Do NOT cancel active downloads here.
        //
        // Per MS-RDPECLIP 2.2.4.1 and 3.1.5.3.2, clipboard locks ensure that
        // the server retains file stream data even after the clipboard changes.
        // Each download holds its own lock (via clipDataId), so the server will
        // continue to service FileContentsRequest PDUs for in-flight transfers
        // regardless of new FormatList arrivals. Downloads complete or fail on
        // their own based on their transfer state and protocol completion.

        // Defense-in-depth: sanitize file info from remote to prevent path traversal.
        // The Rust layer already sanitizes, but we guard again at the JS boundary.
        const sanitized = files.map((f) => ({
            ...f,
            name: RdpFileTransferProvider.sanitizeFileName(f.name),
            path: f.path !== undefined ? RdpFileTransferProvider.sanitizePath(f.path) : undefined,
        }));
        this.availableFiles = sanitized;
        this.clipDataId = clipDataId;
        this.emit('files-available', sanitized);
    }

    /**
     * Extract the basename from a file name, stripping any path traversal or
     * directory components. Returns "unnamed_file" if the name is empty or
     * consists entirely of path separators / traversal sequences.
     */
    /** @internal Visible for testing. */
    static sanitizeFileName(name: string): string {
        // Split on both Windows and Unix separators, find last non-traversal component
        const components = name.split(/[/\\]/);
        for (let i = components.length - 1; i >= 0; i--) {
            const c = components[i];
            if (c.length > 0 && c !== '.' && c !== '..') {
                return c;
            }
        }
        return 'unnamed_file';
    }

    /**
     * Sanitize a relative directory path by stripping traversal components
     * (`.` and `..`) and absolute path prefixes. Returns undefined if the
     * path is empty after sanitization.
     */
    /** @internal Visible for testing. */
    static sanitizePath(path: string): string | undefined {
        const components = path.split(/[/\\]/);
        const safe = components.filter((c) => c.length > 0 && c !== '.' && c !== '..');

        // Strip absolute path prefixes to match the Rust sanitizer's coverage.
        // UNC-like prefix: \\?\ or \\.\ splits into "?" or "." as first component
        if (safe.length > 0 && (safe[0] === '?' || safe[0] === '.')) {
            safe.shift();
            // May be followed by a drive letter (e.g. \\?\C:\path)
            if (safe.length > 0 && /^[A-Za-z]:$/.test(safe[0])) {
                safe.shift();
            }
        }
        // Drive letter prefix: "C:"
        if (safe.length > 0 && /^[A-Za-z]:$/.test(safe[0])) {
            safe.shift();
        }

        if (safe.length === 0) {
            return undefined;
        }

        // Normalize to backslash separator (Windows wire convention)
        return safe.join('\\');
    }

    private handleFileContentsRequest(request: FileContentsRequest): void {
        if (!this.uploadState) {
            if (!this.retainedFiles) {
                console.warn('Received file contents request but no upload in progress');
                this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
                return;
            }
            // Re-paste: rebuild uploadState from retained files so the main
            // code path handles progress/completion tracking identically.
            this.rebuildUploadStateFromRetained();
        }

        // Non-null: either already existed or just rebuilt from retainedFiles
        const state = this.uploadState!;
        const { files, droppedFiles } = state;

        // If this file previously failed (e.g. read timeout), send an error
        // response for any subsequent requests without aborting the batch.
        if (state.failedFiles.has(request.index)) {
            this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
            return;
        }

        const fileHandle = files[request.index];
        const dropped = droppedFiles[request.index];
        if (dropped === undefined) {
            console.error(`File index ${request.index} out of range`);
            this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
            return;
        }

        // Directory entries have no data.  Respond to SIZE with 0 and error
        // for RANGE (the remote should not request ranges for directories).
        if (fileHandle === null || fileHandle === undefined) {
            if ((request.flags & FileContentsFlags.SIZE) !== 0) {
                const sizeBytes = new Uint8Array(8);
                // Size is already 0 in the zeroed buffer
                this.sendSubmitFileContents(request.streamId, false, sizeBytes);
            } else {
                this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
            }
            // Directory entries are not counted toward expectedFileCount,
            // so we do not mark them as completed here.
            return;
        }

        // From here on, fileHandle is a non-null File
        const file: File = fileHandle;

        if ((request.flags & FileContentsFlags.SIZE) !== 0) {
            // SIZE request: return 8-byte LE u64
            const sizeBytes = new Uint8Array(8);
            const view = new DataView(sizeBytes.buffer);
            view.setBigUint64(0, BigInt(file.size), true);
            this.sendSubmitFileContents(request.streamId, false, sizeBytes);
        } else if ((request.flags & FileContentsFlags.RANGE) !== 0) {
            // RANGE request: read file chunk
            const chunk = file.slice(request.position, request.position + request.size);
            const reader = new FileReader();

            // Track active reader by streamId for cleanup on abort/dispose
            // Using streamId (unique per request) instead of file index avoids
            // collisions if the remote sends concurrent requests for the same file.
            state.activeReaders.set(request.streamId, reader);

            // Add timeout to prevent indefinite hangs.
            // On timeout, mark this file as failed and let remaining files continue.
            const timeoutId = setTimeout(() => {
                reader.abort();
                if (this.uploadState !== undefined) {
                    this.uploadState.activeReaders.delete(request.streamId);
                    this.uploadState.readerTimeouts.delete(request.streamId);
                }
                this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
                const err: FileTransferError = {
                    message: `File read timeout after ${RdpFileTransferProvider.FILE_READER_TIMEOUT_MS / 1000}s`,
                    transferId: this.uploadState?.transferIds.get(request.index),
                    fileIndex: request.index,
                    fileName: dropped.name,
                    direction: 'upload',
                };
                this.emit('error', err);

                // Mark this file as failed and check if the batch is done
                if (this.uploadState !== undefined) {
                    this.uploadState.failedFiles.add(request.index);
                    this.uploadState.completedFiles.add(request.index);
                    if (this.uploadState.completedFiles.size >= this.uploadState.expectedFileCount) {
                        const { resolve, droppedFiles: completed } = this.uploadState;
                        this.retainedFiles = completed;
                        this.uploadState = undefined;
                        resolve();
                    }
                }
            }, RdpFileTransferProvider.FILE_READER_TIMEOUT_MS);
            state.readerTimeouts.set(request.streamId, timeoutId);

            reader.onload = () => {
                // Clean up reader and timeout after successful read
                if (this.uploadState !== undefined) {
                    this.uploadState.activeReaders.delete(request.streamId);
                    const timeout = this.uploadState.readerTimeouts.get(request.streamId);
                    if (timeout !== undefined) {
                        clearTimeout(timeout);
                        this.uploadState.readerTimeouts.delete(request.streamId);
                    }
                }

                const data = new Uint8Array(reader.result as ArrayBuffer);
                this.sendSubmitFileContents(request.streamId, false, data);

                // Track cumulative bytes served per file for robust completion
                // detection regardless of chunk request order.
                if (this.uploadState !== undefined) {
                    const served = (this.uploadState.bytesServed.get(request.index) ?? 0) + data.length;
                    this.uploadState.bytesServed.set(request.index, served);

                    // Emit progress (clamp percentage to 100% in case of overlapping ranges)
                    const uploadTransferId = this.uploadState.transferIds.get(request.index) ?? -1;
                    const progress: TransferProgress = {
                        transferId: uploadTransferId,
                        fileIndex: request.index,
                        fileName: dropped.name,
                        bytesTransferred: served,
                        totalBytes: file.size,
                        percentage: file.size === 0 ? 100 : Math.min((served / file.size) * 100, 100),
                    };
                    this.emit('upload-progress', progress);

                    // Check if all bytes for this file have been served
                    if (served >= file.size) {
                        const alreadyCompleted = this.uploadState.completedFiles.has(request.index);

                        this.uploadState.completedFiles.add(request.index);

                        if (!alreadyCompleted) {
                            this.emit('upload-complete', file, request.index, uploadTransferId);
                        }

                        if (this.uploadState.completedFiles.size === this.uploadState.expectedFileCount) {
                            // All files uploaded successfully. Retain DroppedFile
                            // metadata so re-paste from the remote can serve data again.
                            const { resolve, droppedFiles: completed } = this.uploadState;
                            this.retainedFiles = completed;
                            this.uploadState = undefined;
                            resolve();
                        }
                    }
                }
            };

            reader.onerror = () => {
                // Clean up reader and timeout after error
                if (this.uploadState !== undefined) {
                    this.uploadState.activeReaders.delete(request.streamId);
                    const timeout = this.uploadState.readerTimeouts.get(request.streamId);
                    if (timeout !== undefined) {
                        clearTimeout(timeout);
                        this.uploadState.readerTimeouts.delete(request.streamId);
                    }
                }

                // If this file already failed (e.g. timeout), the timeout
                // handler already tracked completion.
                if (this.uploadState?.failedFiles.has(request.index) === true) {
                    return;
                }

                this.sendSubmitFileContents(request.streamId, true, new Uint8Array());
                const err: FileTransferError = {
                    message: 'Failed to read file chunk',
                    transferId: this.uploadState?.transferIds.get(request.index),
                    fileIndex: request.index,
                    fileName: dropped.name,
                    direction: 'upload',
                    cause: reader.error,
                };
                this.emit('error', err);

                // Mark this file as failed and let remaining files continue.
                if (this.uploadState !== undefined) {
                    this.uploadState.failedFiles.add(request.index);
                    this.uploadState.completedFiles.add(request.index);
                    if (this.uploadState.completedFiles.size >= this.uploadState.expectedFileCount) {
                        const { resolve, droppedFiles: completed } = this.uploadState;
                        this.retainedFiles = completed;
                        this.uploadState = undefined;
                        resolve();
                    }
                }
            };

            reader.readAsArrayBuffer(chunk);
        }
    }

    /**
     * Lazily rebuild uploadState from retainedFiles when the remote re-pastes
     * after the original upload completed. This lets the main code path in
     * handleFileContentsRequest handle progress and completion identically.
     * No external promise - resolve/reject are no-ops.
     */
    private rebuildUploadStateFromRetained(): void {
        const dropped = this.retainedFiles!;
        const transferIds = new Map<number, number>();
        for (let i = 0; i < dropped.length; i++) {
            transferIds.set(i, this.generateStreamId());
        }

        const fileCount = dropped.filter((d) => d.isDirectory !== true).length;

        this.uploadState = {
            files: dropped.map((d) => d.file),
            droppedFiles: dropped,
            failedFiles: new Set(),
            expectedFileCount: fileCount,
            completedFiles: new Set(),
            bytesServed: new Map(),
            activeReaders: new Map(),
            readerTimeouts: new Map(),
            transferIds,
            resolve: () => {},
            reject: () => {},
            isRePaste: true,
        };

        this.emit('upload-batch-started', transferIds, dropped);
    }

    private handleFileContentsResponse(response: FileContentsResponse): void {
        const state = this.activeDownloads.get(response.streamId);
        if (!state) {
            console.warn(`Received response for unknown stream ${response.streamId}`);
            return;
        }

        if (response.isError) {
            this.activeDownloads.delete(response.streamId);
            state.chunks = [];
            const err: FileTransferError = {
                message: 'Remote failed to provide file contents',
                transferId: state.streamId,
                fileIndex: state.fileIndex,
                fileName: state.fileInfo.name,
                direction: 'download',
            };
            this.emit('error', err);
            state.reject(new Error(err.message));
            return;
        }

        if (state.expectedSize === undefined) {
            // This is the SIZE response
            // Validate response data is valid before creating DataView
            if (response.data.length < 8) {
                this.activeDownloads.delete(response.streamId);
                state.chunks = [];
                const err: FileTransferError = {
                    message: 'Invalid SIZE response: expected 8 bytes for file size',
                    transferId: state.streamId,
                    fileIndex: state.fileIndex,
                    fileName: state.fileInfo.name,
                    direction: 'download',
                };
                this.emit('error', err);
                state.reject(new Error(err.message));
                return;
            }

            const view = new DataView(response.data.buffer, response.data.byteOffset, response.data.byteLength);
            const size = Number(view.getBigUint64(0, true));

            // Validate file size doesn't exceed browser memory limits
            if (size > RdpFileTransferProvider.MAX_FILE_SIZE) {
                this.activeDownloads.delete(response.streamId);
                state.chunks = [];
                const err: FileTransferError = {
                    message: `File size ${(size / (1024 * 1024 * 1024)).toFixed(2)}GB exceeds maximum download limit of 2GB`,
                    transferId: state.streamId,
                    fileIndex: state.fileIndex,
                    fileName: state.fileInfo.name,
                    direction: 'download',
                };
                this.emit('error', err);
                state.reject(new Error(err.message));
                return;
            }

            state.expectedSize = size;

            // Handle empty files
            if (size === 0) {
                this.activeDownloads.delete(response.streamId);
                const blob = new Blob([]);
                this.emit('download-complete', state.fileInfo, blob, state.fileIndex, state.streamId);
                state.resolve(blob);
                return;
            }

            // Request data in chunks
            this.requestNextChunk(state);
        } else {
            // This is a DATA response.
            // TODO: chunks accumulate in memory until the download completes and a
            // Blob is created. For a 2 GB file this means ~4 GB peak RAM (chunks +
            // final Blob). Consider incremental Blob construction or the File System
            // Access API (WritableStream) to reduce peak memory in a future milestone.
            state.chunks.push(response.data);
            state.bytesReceived += response.data.length;

            // Validate that received data doesn't grossly exceed expected size
            if (state.bytesReceived > state.expectedSize * 2) {
                this.activeDownloads.delete(response.streamId);
                state.chunks = [];
                const err: FileTransferError = {
                    message: `Received ${state.bytesReceived} bytes but expected ${state.expectedSize} — aborting`,
                    transferId: state.streamId,
                    fileIndex: state.fileIndex,
                    fileName: state.fileInfo.name,
                    direction: 'download',
                };
                this.emit('error', err);
                state.reject(new Error(err.message));
                return;
            }

            // Emit progress (clamp percentage to 100% in case server sends slightly more than expected)
            const progress: TransferProgress = {
                transferId: state.streamId,
                fileIndex: state.fileIndex,
                fileName: state.fileInfo.name,
                bytesTransferred: state.bytesReceived,
                totalBytes: state.expectedSize,
                percentage: Math.min((state.bytesReceived / state.expectedSize) * 100, 100),
            };
            this.emit('download-progress', progress);

            // Check if download complete
            if (state.bytesReceived >= state.expectedSize) {
                this.activeDownloads.delete(response.streamId);
                const blob = new Blob(state.chunks as BlobPart[]);
                this.emit('download-complete', state.fileInfo, blob, state.fileIndex, state.streamId);
                state.resolve(blob);
            } else {
                // Request next chunk
                this.requestNextChunk(state);
            }
        }
    }

    private handleLock(_dataId: number): void {
        // Remote locked their clipboard (informational only for uploads).
    }

    private handleUnlock(_dataId: number): void {
        // Remote unlocked their clipboard (informational only for uploads).
    }

    private handleLocksExpired(clipDataIds: Uint32Array): void {
        // Client-side locks expired due to inactivity timeout
        // Check which active downloads are affected and abort them
        const expiredLockSet = new Set<number>();
        for (let i = 0; i < clipDataIds.length; i++) {
            expiredLockSet.add(clipDataIds[i]);
        }

        // Abort downloads that are using expired locks
        for (const [streamId, state] of this.activeDownloads) {
            if (state.clipDataId !== undefined && expiredLockSet.has(state.clipDataId)) {
                this.activeDownloads.delete(streamId);
                state.chunks = [];

                // Build user-friendly error message with timeout info and remediation
                const errorMessage =
                    `File download timed out for "${state.fileInfo.name}". ` +
                    `Clipboard lock expired due to inactivity. ` +
                    `This can happen with slow network connections or large files. ` +
                    `Try downloading smaller files, increasing chunk size, or checking your network connection.`;

                state.reject(new Error(errorMessage));

                this.emit('error', {
                    message: errorMessage,
                    transferId: state.streamId,
                    fileIndex: state.fileIndex,
                    fileName: state.fileInfo.name,
                    direction: 'download',
                });
            }
        }
    }

    // ==================== Helper Methods ====================

    private requestNextChunk(state: TransferState): void {
        if (state.expectedSize === undefined || state.expectedSize === 0) {
            return;
        }

        const position = state.bytesReceived;
        const remaining = state.expectedSize - position;
        const size = Math.min(this.chunkSize, remaining);

        try {
            this.sendRequestFileContents(
                state.streamId,
                state.fileIndex,
                FileContentsFlags.RANGE,
                position,
                size,
                state.clipDataId,
            );
        } catch (error) {
            this.activeDownloads.delete(state.streamId);
            state.chunks = [];
            const err: FileTransferError = {
                message: 'Failed to request file chunk',
                transferId: state.streamId,
                fileIndex: state.fileIndex,
                fileName: state.fileInfo.name,
                direction: 'download',
                cause: error,
            };
            this.emit('error', err);
            state.reject(new Error(err.message, { cause: error }));
        }
    }

    private generateStreamId(): number {
        // Monotonic counter wrapping at 32-bit boundary.
        // After wraparound (~4 billion IDs), skip any IDs still in activeDownloads
        // to avoid silently overwriting an in-flight download's state.
        const maxAttempts = this.activeDownloads.size + 2;
        for (let i = 0; i < maxAttempts; i++) {
            const streamId = this.nextStreamId;
            this.nextStreamId = (this.nextStreamId + 1) % 0x1_0000_0000; // Wrap at 2^32
            if (this.nextStreamId === 0) {
                this.nextStreamId = 1; // Skip 0
            }
            if (!this.activeDownloads.has(streamId) && streamId !== 0) {
                return streamId;
            }
        }
        // Should never happen: more active downloads than the counter can skip
        throw new Error('unable to generate unique stream ID');
    }
}
