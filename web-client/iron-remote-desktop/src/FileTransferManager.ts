import type { Session } from './interfaces/Session';
import type { SessionBuilder } from './interfaces/SessionBuilder';
import type { FileInfo, FileContentsRequest, FileContentsResponse } from './interfaces/FileTransfer';
import { FileContentsFlags } from './enums/FileContentsFlags';

/**
 * Configuration options for FileTransferManager.
 */
export interface FileTransferManagerOptions {
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
     * Called when an upload finishes (success, failure, cancellation, or dispose).
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
 * Information emitted when a transfer is cancelled.
 */
export interface TransferCancellation {
    /** Unique transfer identifier. -1 for batch-level upload cancellation. */
    transferId: number;
    /** File index in the transfer list. -1 for batch-level upload cancellation. */
    fileIndex: number;
    /** Direction of the cancelled transfer */
    direction: 'download' | 'upload';
}

/**
 * Result of initiating a file download via {@link FileTransferManager.downloadFile}.
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
 * Result of initiating a file upload via {@link FileTransferManager.uploadFiles}.
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
    signal?: AbortSignal;
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
    signal?: AbortSignal;
    /** Per-file abort signals keyed by file index.  When a signal fires the
     *  manager sends an error response for subsequent chunk requests on that
     *  file and marks it as cancelled without aborting the entire batch. */
    perFileSignals?: Map<number, AbortSignal>;
    /** File indices whose per-file signal has been aborted. */
    cancelledFiles: Set<number>;
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
    'transfer-cancelled': [TransferCancellation];
    error: [FileTransferError];
};

/**
 * FileTransferManager provides a high-level API for bidirectional file transfer
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
 * is acquired automatically. The lock ID is passed to FileTransferManager via
 * the filesAvailable callback and used in all subsequent FileContentsRequest
 * PDUs. Lock lifecycle (expiry on clipboard change, Unlock PDU emission) is
 * handled entirely by the Rust layer - no explicit lock/unlock calls are needed.
 *
 * ## Error Handling Best Practices
 *
 * Production applications should implement comprehensive error handling:
 *
 * @example Basic Error Handling
 * ```typescript
 * const manager = FileTransferManager.setup(builder);
 *
 * // Handle transfer errors
 * manager.on('error', (error) => {
 *   console.error('Transfer error:', error.message);
 *   if (error.fileName) {
 *     showNotification(`Failed to transfer ${error.fileName}: ${error.message}`);
 *   }
 * });
 *
 * // Download with error handling
 * manager.on('files-available', async (files) => {
 *   for (const file of files) {
 *     try {
 *       const blob = await manager.downloadFile(file, files.indexOf(file));
 *       saveAs(blob, file.name);
 *     } catch (error) {
 *       console.error(`Failed to download ${file.name}:`, error);
 *       // Handle error in UI
 *     }
 *   }
 * });
 * ```
 *
 * @example Using AbortController for Timeouts
 * ```typescript
 * const controller = new AbortController();
 * const timeoutId = setTimeout(() => controller.abort(), 60000); // 60s timeout
 *
 * try {
 *   const blob = await manager.downloadFile(fileInfo, 0, controller.signal);
 *   clearTimeout(timeoutId);
 *   saveAs(blob, fileInfo.name);
 * } catch (error) {
 *   clearTimeout(timeoutId);
 *   if (error.message === 'Download cancelled') {
 *     console.log('Download timed out after 60 seconds');
 *   } else {
 *     console.error('Download failed:', error);
 *   }
 * }
 * ```
 *
 * @example Retry Logic with Exponential Backoff
 * ```typescript
 * async function downloadWithRetry(manager, fileInfo, fileIndex, maxRetries = 3) {
 *   for (let attempt = 0; attempt < maxRetries; attempt++) {
 *     try {
 *       return await manager.downloadFile(fileInfo, fileIndex);
 *     } catch (error) {
 *       // Don't retry if explicitly cancelled
 *       if (error.message === 'Download cancelled') {
 *         throw error;
 *       }
 *
 *       // Last attempt - give up
 *       if (attempt === maxRetries - 1) {
 *         throw new Error(`Download failed after ${maxRetries} attempts: ${error.message}`);
 *       }
 *
 *       // Exponential backoff: 1s, 2s, 4s
 *       const delay = Math.pow(2, attempt) * 1000;
 *       console.log(`Retry ${attempt + 1}/${maxRetries} after ${delay}ms...`);
 *       await new Promise(resolve => setTimeout(resolve, delay));
 *     }
 *   }
 * }
 * ```
 *
 * @example Handling Lock Expiration
 * ```typescript
 * manager.on('error', (error) => {
 *   // Lock expiration errors contain specific guidance
 *   if (error.message.includes('lock expired')) {
 *     console.warn('Transfer interrupted due to lock expiration');
 *     console.warn('This can happen with slow networks or large files');
 *     console.warn('Consider increasing chunk size or checking network connection');
 *
 *     // Show user-friendly error
 *     showNotification(
 *       'File download timed out',
 *       'The transfer took too long and was cancelled. Try a faster connection or smaller files.',
 *       'warning'
 *     );
 *   }
 * });
 * ```
 *
 * @example Basic Usage
 * ```typescript
 * const builder = new SessionBuilder();
 * const manager = FileTransferManager.setup(builder, { chunkSize: 64 * 1024 });
 * const session = await builder.connect();
 *
 * // Handle downloads from remote
 * manager.on('files-available', async (files) => {
 *   for (const file of files) {
 *     const blob = await manager.downloadFile(file, files.indexOf(file));
 *     saveAs(blob, file.name);
 *   }
 * });
 *
 * // Handle uploads using file picker
 * const files = await manager.showFilePicker({ multiple: true });
 * await manager.uploadFiles(files);
 * ```
 */
export class FileTransferManager {
    /** Maximum file size for downloads (2GB) to prevent browser out-of-memory errors */
    private static readonly MAX_FILE_SIZE = 2 * 1024 * 1024 * 1024;
    /** Timeout for FileReader operations (60 seconds) to prevent stalled uploads */
    private static readonly FILE_READER_TIMEOUT_MS = 60 * 1000;
    /** Maximum recursion depth when traversing dropped directories. */
    private static readonly MAX_DIRECTORY_DEPTH = 32;
    /** Maximum total entries (files + directories) collected from a single drop. */
    private static readonly MAX_DIRECTORY_ENTRIES = 1000;

    private session?: Session;
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

    // Stored references for restoring the original builder.connect() on dispose
    private registeredBuilder?: SessionBuilder;
    private originalConnect?: () => Promise<Session>;

    constructor(options?: FileTransferManagerOptions) {
        this.chunkSize = options?.chunkSize ?? 65536; // Default: 64KB
        this.onUploadStarted = options?.onUploadStarted;
        this.onUploadFinished = options?.onUploadFinished;
    }

    /**
     * Set the session instance after connection is established.
     * This is called automatically by registerCallbacks after builder.connect().
     *
     * @param session - The connected session instance
     */
    private setSession(session: Session): void {
        this.session = session;
    }

    private ensureSession(): Session {
        if (this.session === undefined) {
            throw new Error('FileTransferManager: Session not available. Ensure builder.connect() has been called.');
        }
        return this.session;
    }

    /**
     * Create and configure a FileTransferManager for use with a SessionBuilder.
     * This is the recommended way to use FileTransferManager.
     *
     * @param builder - SessionBuilder instance to register callbacks on
     * @param options - Optional configuration for the manager
     * @returns Configured FileTransferManager instance ready to use after builder.connect()
     *
     * @example
     * ```typescript
     * const builder = new IronRemoteDesktop.SessionBuilder();
     * const manager = FileTransferManager.setup(builder, { chunkSize: 64 * 1024 });
     * const session = await builder.connect(); // Manager is automatically ready
     *
     * manager.on('files-available', async (files) => {
     *   const blob = await manager.downloadFile(files[0], 0);
     *   saveAs(blob, files[0].name);
     * });
     * ```
     */
    static setup(builder: SessionBuilder, options?: FileTransferManagerOptions): FileTransferManager {
        const manager = new FileTransferManager(options);
        FileTransferManager.registerCallbacks(builder, manager);
        return manager;
    }

    /**
     * Register callbacks with SessionBuilder and automatically set session after connection.
     *
     * Note: Most developers should use FileTransferManager.setup() instead, which calls this internally.
     *
     * @param builder - SessionBuilder instance to register callbacks on
     * @param manager - FileTransferManager instance to receive callbacks
     *
     * @example
     * ```typescript
     * const manager = new FileTransferManager({ chunkSize: 64 * 1024 });
     * const builder = new IronRemoteDesktop.SessionBuilder();
     * FileTransferManager.registerCallbacks(builder, manager);
     * const session = await builder.connect(); // Session automatically set on manager
     * ```
     */
    static registerCallbacks(builder: SessionBuilder, manager: FileTransferManager): void {
        builder.filesAvailableCallback((files, clipDataId) => manager.handleFilesAvailable(files, clipDataId));
        builder.fileContentsRequestCallback((req) => manager.handleFileContentsRequest(req));
        builder.fileContentsResponseCallback((resp) => manager.handleFileContentsResponse(resp));
        builder.lockCallback((id) => manager.handleLock(id));
        builder.unlockCallback((id) => manager.handleUnlock(id));
        builder.locksExpiredCallback((ids) => manager.handleLocksExpired(ids));

        // Wrap connect() to automatically set session on manager after connection.
        // Store references so dispose() can restore the original.
        const originalConnect = builder.connect.bind(builder);
        manager.registeredBuilder = builder;
        manager.originalConnect = originalConnect;
        builder.connect = async function (): Promise<Session> {
            if (manager.disposed) {
                return originalConnect();
            }
            const session = await originalConnect();
            manager.setSession(session);
            return session;
        };
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
     * @param signal - Optional AbortSignal for cancellation
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
    downloadFile(fileInfo: FileInfo, fileIndex: number, signal?: AbortSignal): DownloadHandle {
        // Generate unique stream ID (serves as transferId)
        const streamId = this.generateStreamId();

        const completion = this.executeDownload(fileInfo, fileIndex, streamId, signal);

        return { transferId: streamId, completion };
    }

    /**
     * Internal: execute the async download workflow for a single file.
     */
    private async executeDownload(
        fileInfo: FileInfo,
        fileIndex: number,
        streamId: number,
        signal?: AbortSignal,
    ): Promise<Blob> {
        // Check if already cancelled
        if (signal?.aborted === true) {
            throw new Error('Download cancelled');
        }

        // Use the clipboard lock acquired by the Rust layer when the file list
        // was received. The lock lifecycle (creation, expiry, Unlock PDUs) is
        // managed entirely by the cliprdr processor - we just pass the ID
        // through to FileContentsRequest so the server associates requests with
        // the correct clipboard snapshot.
        const clipDataId = this.clipDataId;

        // Create internal AbortController to clean up the abort listener when the download settles
        const internalAc = new AbortController();

        // Create transfer state
        const transferPromise = new Promise<Blob>((resolve, reject) => {
            const wrappedResolve = (blob: Blob): void => {
                internalAc.abort(); // Remove abort listener
                resolve(blob);
            };
            const wrappedReject = (error: Error): void => {
                internalAc.abort(); // Remove abort listener
                reject(error);
            };

            const state: TransferState = {
                fileInfo,
                fileIndex,
                streamId,
                clipDataId,
                chunks: [],
                bytesReceived: 0,
                signal,
                resolve: wrappedResolve,
                reject: wrappedReject,
            };

            this.activeDownloads.set(streamId, state);

            // Handle abort signal
            if (signal !== undefined) {
                signal.addEventListener(
                    'abort',
                    () => {
                        // Guard: only act if this download is still active
                        if (!this.activeDownloads.has(streamId)) return;

                        this.activeDownloads.delete(streamId);
                        this.emit('transfer-cancelled', {
                            transferId: streamId,
                            fileIndex,
                            direction: 'download',
                        });
                        wrappedReject(new Error('Download cancelled'));
                    },
                    { signal: internalAc.signal },
                );
            }
        });

        // Request file size first (flags = 0x1).
        // Per MS-RDPECLIP 2.2.5.3, SIZE requests MUST set cbRequested to 8.
        try {
            this.ensureSession().requestFileContents(streamId, fileIndex, FileContentsFlags.SIZE, 0, 8, clipDataId);
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
     * @param signal - Optional AbortSignal for cancellation
     * @returns AsyncGenerator yielding file/blob pairs as they complete
     *
     * @example
     * ```typescript
     * for await (const { file, blob } of manager.downloadFiles(files)) {
     *   saveAs(blob, file.name);
     * }
     * ```
     */
    async *downloadFiles(
        files: FileInfo[],
        signal?: AbortSignal,
    ): AsyncGenerator<{ file: FileInfo; blob: Blob; transferId: number }> {
        for (let i = 0; i < files.length; i++) {
            if (signal?.aborted === true) {
                throw new Error('Download cancelled');
            }

            const file = files[i];
            const { transferId, completion } = this.downloadFile(file, i, signal);
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
     * @param options.signal - Optional AbortSignal for cancellation
     * @returns Promise resolving to map of fileIndex to Blob
     *
     * @example
     * ```typescript
     * const blobs = await manager.downloadFilesConcurrent(files, {
     *   maxConcurrent: 5,
     *   signal: abortController.signal
     * });
     * files.forEach((file, i) => saveAs(blobs.get(i)!, file.name));
     * ```
     */
    async downloadFilesConcurrent(
        files: FileInfo[],
        options: { maxConcurrent?: number; signal?: AbortSignal } = {},
    ): Promise<Map<number, Blob>> {
        const maxConcurrent = options.maxConcurrent ?? 3;
        const signal = options.signal;

        if (signal?.aborted === true) {
            throw new Error('Download cancelled');
        }

        const results = new Map<number, Blob>();
        const errors: Array<{ index: number; error: unknown }> = [];

        // Create download tasks
        const downloadTasks = files.map((file, index) => async () => {
            try {
                const { completion } = this.downloadFile(file, index, signal);
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
            if (signal !== undefined && signal.aborted) {
                break;
            }

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
     * @param signal - Optional AbortSignal for batch-level cancellation (aborts all files)
     * @param perFileSignals - Optional map of file index to AbortSignal for per-file cancellation.
     *   When a per-file signal aborts, that file is skipped/cancelled without aborting the batch.
     * @returns Handle with synchronous transferIds and async completion
     *
     * @example
     * ```typescript
     * const files = await manager.showFilePicker({ multiple: true });
     * const { transferIds, completion } = manager.uploadFiles(files);
     * // transferIds available immediately for UI binding
     * await completion;
     * ```
     *
     * @example Per-file cancellation
     * ```typescript
     * const controllers = files.map(() => new AbortController());
     * const perFileSignals = new Map(controllers.map((c, i) => [i, c.signal]));
     * const { transferIds, completion } = manager.uploadFiles(files, batchSignal, perFileSignals);
     * // Cancel just file 0:
     * controllers[0].abort();
     * await completion;
     * ```
     */
    uploadFiles(
        files: File[] | DroppedFile[],
        signal?: AbortSignal,
        perFileSignals?: Map<number, AbortSignal>,
    ): UploadHandle {
        if (signal?.aborted === true) {
            throw new Error('Upload cancelled');
        }

        if (this.uploadState !== undefined) {
            throw new Error('Upload already in progress');
        }

        // New upload supersedes any retained files from a previous batch
        this.retainedFiles = undefined;

        // Normalize: accept both plain File[] (backward compat) and DroppedFile[]
        const dropped: DroppedFile[] = FileTransferManager.normalizeToDroppedFiles(files);

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

        // Internal AbortController to clean up the abort listener when the upload settles
        // (success, error, or cancellation). This prevents the listener from firing after
        // the upload promise has already resolved/rejected.
        const internalAc = new AbortController();

        // Create completion promise
        const completion = new Promise<void>((resolve, reject) => {
            const wrappedResolve = (): void => {
                internalAc.abort(); // Remove abort listener
                resolve();
            };
            const wrappedReject = (error: Error): void => {
                internalAc.abort(); // Remove abort listener
                reject(error);
            };

            // Store upload state with completion tracking
            this.uploadState = {
                files: fileHandles,
                droppedFiles: dropped,
                signal,
                perFileSignals,
                cancelledFiles: new Set(),
                expectedFileCount: fileCount,
                completedFiles: new Set(),
                bytesServed: new Map(),
                activeReaders: new Map(),
                readerTimeouts: new Map(),
                transferIds,
                resolve: wrappedResolve,
                reject: wrappedReject,
            };

            // Handle abort signal
            if (signal !== undefined) {
                signal.addEventListener(
                    'abort',
                    () => {
                        // Guard: only act if upload is still in progress
                        if (this.uploadState === undefined) return;

                        // Abort all active FileReaders and clear timeouts
                        for (const reader of this.uploadState.activeReaders.values()) {
                            reader.abort();
                        }
                        this.uploadState.activeReaders.clear();

                        for (const timeout of this.uploadState.readerTimeouts.values()) {
                            clearTimeout(timeout);
                        }
                        this.uploadState.readerTimeouts.clear();

                        this.uploadState = undefined;
                        this.onUploadFinished?.();
                        this.emit('transfer-cancelled', {
                            transferId: -1,
                            fileIndex: -1,
                            direction: 'upload',
                        });
                        wrappedReject(new Error('Upload cancelled'));
                    },
                    { signal: internalAc.signal },
                );
            }

            // Register per-file abort listeners
            if (perFileSignals !== undefined) {
                for (const [fileIndex, fileSignal] of perFileSignals) {
                    fileSignal.addEventListener(
                        'abort',
                        () => {
                            if (this.uploadState === undefined) return;
                            if (this.uploadState.cancelledFiles.has(fileIndex)) return;
                            if (this.uploadState.completedFiles.has(fileIndex)) return;

                            this.uploadState.cancelledFiles.add(fileIndex);

                            // Abort any active FileReader for this file
                            const reader = this.uploadState.activeReaders.get(fileIndex);
                            if (reader !== undefined) {
                                reader.abort();
                                this.uploadState.activeReaders.delete(fileIndex);
                            }
                            const timeout = this.uploadState.readerTimeouts.get(fileIndex);
                            if (timeout !== undefined) {
                                clearTimeout(timeout);
                                this.uploadState.readerTimeouts.delete(fileIndex);
                            }

                            this.emit('transfer-cancelled', {
                                transferId: this.uploadState.transferIds.get(fileIndex) ?? -1,
                                fileIndex,
                                direction: 'upload',
                            });

                            // Mark as completed and check if the whole batch is done
                            this.uploadState.completedFiles.add(fileIndex);
                            if (this.uploadState.completedFiles.size >= this.uploadState.expectedFileCount) {
                                this.uploadState = undefined;
                                this.onUploadFinished?.();
                                wrappedResolve();
                            }
                        },
                        { signal: internalAc.signal },
                    );
                }
            }

            // Suppress clipboard monitoring while the upload is in flight so the
            // 100ms polling loop does not clobber our FormatList with a text/image update.
            this.onUploadStarted?.();

            // Initiate file copy (broadcasts file list to remote)
            try {
                this.ensureSession().initiateFileCopy(fileInfos);
                this.emit('upload-batch-started', transferIds, dropped);
            } catch (error) {
                this.uploadState = undefined;
                this.onUploadFinished?.();
                const err: FileTransferError = {
                    message: 'Failed to initiate file upload',
                    direction: 'upload',
                    cause: error,
                };
                this.emit('error', err);
                wrappedReject(new Error(err.message, { cause: error }));
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
        if (results.length >= FileTransferManager.MAX_DIRECTORY_ENTRIES) return;

        if (depth > FileTransferManager.MAX_DIRECTORY_DEPTH) {
            console.warn(
                `Skipping "${entry.name}": directory depth exceeds ${FileTransferManager.MAX_DIRECTORY_DEPTH}`,
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
            const children = await FileTransferManager.readAllDirectoryEntries(reader);
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
     * Call this when the session is terminating or FileTransferManager is no longer needed.
     */
    dispose(): void {
        this.disposed = true;

        // Cancel active downloads (lock cleanup is handled by the Rust layer)
        for (const state of this.activeDownloads.values()) {
            state.chunks = [];
            state.reject(new Error('FileTransferManager disposed'));
        }
        this.activeDownloads.clear();

        // Abort active FileReaders, clear timeouts, and reject upload promise
        if (this.uploadState !== undefined) {
            for (const timeout of this.uploadState.readerTimeouts.values()) {
                clearTimeout(timeout);
            }
            this.uploadState.readerTimeouts.clear();

            for (const reader of this.uploadState.activeReaders.values()) {
                reader.abort();
            }
            this.uploadState.activeReaders.clear();

            this.uploadState.reject(new Error('FileTransferManager disposed'));
        }
        const hadUpload = this.uploadState !== undefined;
        this.uploadState = undefined;
        this.retainedFiles = undefined;
        if (hadUpload) {
            this.onUploadFinished?.();
        }

        // Restore the original builder.connect() to prevent the disposed manager
        // from intercepting future connections
        if (this.registeredBuilder !== undefined && this.originalConnect !== undefined) {
            this.registeredBuilder.connect = this.originalConnect;
            this.registeredBuilder = undefined;
            this.originalConnect = undefined;
        }

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
        // their own; the user can also abort them via the cancel button.

        // Defense-in-depth: sanitize file info from remote to prevent path traversal.
        // The Rust layer already sanitizes, but we guard again at the JS boundary.
        const sanitized = files.map((f) => ({
            ...f,
            name: FileTransferManager.sanitizeFileName(f.name),
            path: f.path !== undefined ? FileTransferManager.sanitizePath(f.path) : undefined,
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

        // Strip absolute path prefix: drive letter like "C:"
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
                this.session?.submitFileContents(request.streamId, true, new Uint8Array());
                return;
            }
            // Re-paste: rebuild uploadState from retained files so the main
            // code path handles progress/completion tracking identically.
            this.rebuildUploadStateFromRetained();
        }

        // Non-null: either already existed or just rebuilt from retainedFiles
        const state = this.uploadState!;
        const { files, droppedFiles, signal } = state;

        if (signal?.aborted === true) {
            // Send error response
            this.session?.submitFileContents(request.streamId, true, new Uint8Array());
            return;
        }

        // Per-file cancellation: if this file was individually cancelled,
        // send an error response and mark it completed without aborting the batch.
        if (state.cancelledFiles.has(request.index)) {
            this.session?.submitFileContents(request.streamId, true, new Uint8Array());
            return;
        }

        const fileHandle = files[request.index];
        const dropped = droppedFiles[request.index];
        if (dropped === undefined) {
            console.error(`File index ${request.index} out of range`);
            this.session?.submitFileContents(request.streamId, true, new Uint8Array());
            return;
        }

        // Directory entries have no data.  Respond to SIZE with 0 and error
        // for RANGE (the remote should not request ranges for directories).
        if (fileHandle === null || fileHandle === undefined) {
            if ((request.flags & FileContentsFlags.SIZE) !== 0) {
                const sizeBytes = new Uint8Array(8);
                // Size is already 0 in the zeroed buffer
                this.session?.submitFileContents(request.streamId, false, sizeBytes);
            } else {
                this.session?.submitFileContents(request.streamId, true, new Uint8Array());
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
            this.session?.submitFileContents(request.streamId, false, sizeBytes);
        } else if ((request.flags & FileContentsFlags.RANGE) !== 0) {
            // RANGE request: read file chunk
            const chunk = file.slice(request.position, request.position + request.size);
            const reader = new FileReader();

            // Track active reader by streamId for cleanup on abort/dispose
            // Using streamId (unique per request) instead of file index avoids
            // collisions if the remote sends concurrent requests for the same file.
            state.activeReaders.set(request.streamId, reader);

            // Add timeout to prevent indefinite hangs.
            // On timeout, mark this file as failed and let remaining files continue
            // (consistent with per-file cancellation behavior).
            const timeoutId = setTimeout(() => {
                reader.abort();
                if (this.uploadState !== undefined) {
                    this.uploadState.activeReaders.delete(request.streamId);
                    this.uploadState.readerTimeouts.delete(request.streamId);
                }
                this.session?.submitFileContents(request.streamId, true, new Uint8Array());
                const err: FileTransferError = {
                    message: `File read timeout after ${FileTransferManager.FILE_READER_TIMEOUT_MS / 1000}s`,
                    transferId: this.uploadState?.transferIds.get(request.index),
                    fileIndex: request.index,
                    fileName: dropped.name,
                    direction: 'upload',
                };
                this.emit('error', err);

                // Mark this file as failed and check if the batch is done
                if (this.uploadState !== undefined) {
                    this.uploadState.cancelledFiles.add(request.index);
                    this.uploadState.completedFiles.add(request.index);
                    if (this.uploadState.completedFiles.size >= this.uploadState.expectedFileCount) {
                        const { resolve, isRePaste } = this.uploadState;
                        this.uploadState = undefined;
                        if (isRePaste !== true) {
                            this.onUploadFinished?.();
                        }
                        resolve();
                    }
                }
            }, FileTransferManager.FILE_READER_TIMEOUT_MS);
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
                this.session?.submitFileContents(request.streamId, false, data);

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
                            const { resolve, droppedFiles: completed, isRePaste } = this.uploadState;
                            this.retainedFiles = completed;
                            this.uploadState = undefined;
                            if (isRePaste !== true) {
                                this.onUploadFinished?.();
                            }
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

                // If this file was individually cancelled (or already failed via
                // timeout), the per-file handler already tracked completion.
                if (this.uploadState?.cancelledFiles.has(request.index) === true) {
                    return;
                }

                this.session?.submitFileContents(request.streamId, true, new Uint8Array());
                const err: FileTransferError = {
                    message: 'Failed to read file chunk',
                    transferId: this.uploadState?.transferIds.get(request.index),
                    fileIndex: request.index,
                    fileName: dropped.name,
                    direction: 'upload',
                    cause: reader.error,
                };
                this.emit('error', err);

                // Mark this file as failed and let remaining files continue
                // (consistent with per-file cancellation and timeout behavior).
                if (this.uploadState !== undefined) {
                    this.uploadState.cancelledFiles.add(request.index);
                    this.uploadState.completedFiles.add(request.index);
                    if (this.uploadState.completedFiles.size >= this.uploadState.expectedFileCount) {
                        const { resolve, isRePaste } = this.uploadState;
                        this.uploadState = undefined;
                        if (isRePaste !== true) {
                            this.onUploadFinished?.();
                        }
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
     * No abort signals or external promise - resolve/reject are no-ops.
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
            cancelledFiles: new Set(),
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

        if (state.signal?.aborted === true) {
            this.activeDownloads.delete(response.streamId);
            state.chunks = [];
            state.reject(new Error('Download cancelled'));
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
            if (size > FileTransferManager.MAX_FILE_SIZE) {
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
            this.ensureSession().requestFileContents(
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
