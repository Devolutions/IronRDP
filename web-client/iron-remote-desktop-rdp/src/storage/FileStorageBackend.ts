/**
 * A write handle for streaming file data to a storage backend.
 *
 * Created once per download via {@link FileStorageBackend.createWriteHandle}.
 * Chunks are appended via {@link write}, and on success {@link finalize}
 * returns the assembled data as a {@link Blob} (or {@link File}, which
 * extends Blob).  On failure or cancellation, {@link abort} releases all
 * resources held by the handle.
 *
 * Implementations may buffer in memory (Blob backend), stream to the
 * Origin Private File System (OPFS backend), or stream to a user-chosen
 * location (future FSAPI backend).
 */
export interface FileWriteHandle {
    /** Append a chunk of data.
     *
     *  Backends may buffer the chunk in memory or flush it to persistent
     *  storage immediately.  The returned promise resolves once the chunk
     *  has been accepted (not necessarily persisted). */
    write(chunk: Uint8Array): Promise<void>;

    /**
     * Finalize the file and return the result as a Blob.
     *
     * For in-memory backends this assembles a Blob from buffered chunks.
     * For persistent backends (e.g. OPFS) this closes the writable stream
     * and returns a {@link File} (which extends Blob) backed by the on-disk
     * data, keeping peak RAM close to zero.
     *
     * After calling finalize the handle must not be reused.
     */
    finalize(): Promise<Blob>;

    /**
     * Discard all written data and release resources.
     *
     * Safe to call multiple times.  After abort the handle must not be
     * reused.
     */
    abort(): Promise<void>;

    /** Number of bytes successfully written so far. */
    readonly bytesWritten: number;
}

/**
 * Pluggable storage backend for file transfer downloads.
 *
 * The backend determines *where* incoming file chunks are buffered during
 * a download.  Protocol-specific file transfer providers delegate all
 * storage concerns to the active backend, keeping download orchestration
 * logic storage-agnostic.
 *
 * Three backends are planned:
 *
 * | Backend | Buffering          | Peak RAM      | Browser support           |
 * |---------|--------------------|---------------|---------------------------|
 * | Blob    | In-memory array    | ~2x file size | Universal                 |
 * | OPFS    | Origin Private FS  | ~chunk size   | Baseline (September 2025) |
 * | FSAPI   | User-chosen file   | ~chunk size   | Chromium-only (future)    |
 */
export interface FileStorageBackend {
    /** Human-readable backend name, used in log messages. */
    readonly name: string;

    /**
     * Create a write handle for a new download.
     *
     * @param fileName   - Sanitized file basename (used for the temp file
     *                     name in persistent backends).
     * @param expectedSize - Expected total size in bytes.  Backends may use
     *                     this for pre-allocation or quota checks.  A value
     *                     of 0 means the size is unknown or the file is
     *                     empty.
     */
    createWriteHandle(fileName: string, expectedSize: number): Promise<FileWriteHandle>;

    /**
     * Release all backend resources.
     *
     * For persistent backends this deletes the session directory and any
     * temp files.  Safe to call multiple times.
     */
    dispose(): Promise<void>;
}
