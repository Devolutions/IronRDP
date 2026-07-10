import type { FileStorageBackend, FileWriteHandle } from './FileStorageBackend';

/**
 * Write handle that streams chunks to a file in the Origin Private File
 * System via {@link FileSystemWritableFileStream}.
 *
 * Each chunk is flushed to disk immediately, so peak RAM stays close to
 * the chunk size regardless of total file size.  On {@link finalize} the
 * stream is closed and a lazy {@link File} reference (which extends
 * {@link Blob}) is returned -- the browser memory-maps reads from OPFS
 * rather than loading the entire file into RAM.
 */
class OpfsWriteHandle implements FileWriteHandle {
    private writable: FileSystemWritableFileStream | undefined;
    private _bytesWritten = 0;
    private finalized = false;

    constructor(
        private readonly fileHandle: FileSystemFileHandle,
        private readonly sessionDir: FileSystemDirectoryHandle,
        private readonly entryName: string,
        writable: FileSystemWritableFileStream,
    ) {
        this.writable = writable;
    }

    get bytesWritten(): number {
        return this._bytesWritten;
    }

    async write(chunk: Uint8Array): Promise<void> {
        if (this.finalized || !this.writable) {
            throw new Error('OpfsWriteHandle: write after finalize/abort');
        }
        await this.writable.write(chunk);
        this._bytesWritten += chunk.length;
    }

    async finalize(): Promise<Blob> {
        if (this.finalized) {
            throw new Error('OpfsWriteHandle: already finalized or aborted');
        }
        this.finalized = true;

        if (this.writable) {
            await this.writable.close();
            this.writable = undefined;
        }

        // getFile() returns a File (extends Blob) backed by OPFS storage.
        // The browser lazily reads from disk -- the file data is NOT copied
        // into RAM here.
        return this.fileHandle.getFile();
    }

    async abort(): Promise<void> {
        if (this.finalized) {
            return;
        }
        this.finalized = true;

        if (this.writable) {
            try {
                await this.writable.abort();
            } catch {
                // Writable may already be closed or errored; ignore.
            }
            this.writable = undefined;
        }

        // Remove the temp file so it does not consume quota.
        try {
            await this.sessionDir.removeEntry(this.entryName);
        } catch {
            // Entry may already be gone (e.g., session dir was deleted).
        }
    }
}

/**
 * Storage backend that streams download chunks to the Origin Private File
 * System (OPFS).
 *
 * OPFS is a browser-provided, origin-scoped file system that requires no
 * user permission prompts and is available on the main thread (async
 * only).  This backend requires the {@link FileSystemWritableFileStream}
 * API, which reached Baseline across all major browsers in September 2025
 * (Chrome 86+, Firefox 111+, Safari 17.2+, Edge 86+).  Older browsers
 * are detected automatically via {@link OpfsStorageBackend.probe} and
 * fall back to the Blob backend.
 *
 * **How it works:**
 * 1. On construction, a per-session subdirectory is created under
 *    `ironrdp-transfers/` in the OPFS root.
 * 2. Each download opens a {@link FileSystemWritableFileStream} inside
 *    that directory and flushes chunks to disk as they arrive.
 * 3. On completion, the stream is closed and a lazy {@link File} handle is
 *    returned.  The File extends Blob, so existing consumers that expect
 *    a Blob work without changes.
 * 4. On dispose, the entire session directory is deleted.
 *
 * **Trade-offs vs Blob backend:**
 * - Peak RAM drops from ~2x file size to ~chunk size (typically 64 KB).
 * - Moderate write latency per chunk (async disk I/O), but the download
 *   is already async and network-bound.
 * - Storage is subject to the origin's quota (typically 60% of disk).
 * - May be unavailable in some private browsing modes.
 *
 * **Construction:** Use the static {@link OpfsStorageBackend.create}
 * factory method.  The constructor is private because initialization
 * requires async OPFS directory setup.
 */
export class OpfsStorageBackend implements FileStorageBackend {
    readonly name = 'opfs';

    /** Sequence counter for generating unique temp file names. */
    private sequence = 0;

    private constructor(
        private readonly opfsRoot: FileSystemDirectoryHandle,
        private sessionDir: FileSystemDirectoryHandle | undefined,
        private readonly sessionId: string,
    ) {}

    /**
     * Create an OPFS backend, including the per-session directory.
     *
     * Call {@link probe} first to verify OPFS is available before
     * constructing -- this factory assumes OPFS works.
     */
    static async create(opfsRoot: FileSystemDirectoryHandle, sessionId?: string): Promise<OpfsStorageBackend> {
        const id = sessionId ?? `s-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        const transfersDir = await opfsRoot.getDirectoryHandle('ironrdp-transfers', { create: true });
        const sessionDir = await transfersDir.getDirectoryHandle(id, { create: true });

        // Best-effort cleanup of orphaned session directories from previous
        // sessions that did not call dispose() (e.g., tab crash, browser
        // force-quit).  Runs asynchronously and never blocks creation.
        void OpfsStorageBackend.cleanupStale(transfersDir, id);

        return new OpfsStorageBackend(opfsRoot, sessionDir, id);
    }

    /** Maximum age (in milliseconds) before a session directory is considered stale. */
    private static readonly STALE_SESSION_THRESHOLD_MS = 24 * 60 * 60 * 1000; // 24 hours

    /**
     * Remove session directories older than {@link STALE_SESSION_THRESHOLD_MS}.
     *
     * Session IDs generated by {@link create} embed a timestamp in the
     * format `s-{Date.now()}-{random}`.  This method parses that timestamp
     * to determine age.  Directories with unparsable names or those
     * belonging to the current session are skipped.
     */
    private static async cleanupStale(
        transfersDir: FileSystemDirectoryHandle,
        currentSessionId: string,
    ): Promise<void> {
        const now = Date.now();
        try {
            for await (const name of transfersDir.keys()) {
                if (name === currentSessionId) {
                    continue;
                }
                // Parse timestamp from the session ID format: s-{timestamp}-{random}.
                const match = /^s-(\d+)-/.exec(name);
                if (!match) {
                    continue;
                }
                const timestamp = Number(match[1]);
                if (now - timestamp > OpfsStorageBackend.STALE_SESSION_THRESHOLD_MS) {
                    try {
                        await transfersDir.removeEntry(name, { recursive: true });
                    } catch {
                        // Ignore per-entry errors (may be in use by another tab).
                    }
                }
            }
        } catch {
            // The transfers directory may have been removed or is inaccessible.
        }
    }

    /**
     * Probe whether OPFS is usable in the current context.
     *
     * Performs a full round-trip: creates a temp file, opens a writable,
     * closes it, and deletes it.  This catches environments where the API
     * exists but throws at runtime (e.g., some private browsing modes).
     */
    static async probe(opfsRoot: FileSystemDirectoryHandle): Promise<boolean> {
        try {
            const handle = await opfsRoot.getFileHandle('.ironrdp-opfs-probe', { create: true });
            const writable = await handle.createWritable();
            await writable.close();
            await opfsRoot.removeEntry('.ironrdp-opfs-probe');
            return true;
        } catch {
            return false;
        }
    }

    async createWriteHandle(fileName: string, _expectedSize: number): Promise<FileWriteHandle> {
        if (!this.sessionDir) {
            throw new Error('OpfsStorageBackend: backend has been disposed');
        }

        // Use a sequence number + sanitized name to avoid collisions when
        // the same file name is downloaded multiple times in one session.
        const seq = this.sequence++;
        const entryName = `${seq}-${sanitizeOpfsName(fileName)}`;

        const fileHandle = await this.sessionDir.getFileHandle(entryName, { create: true });
        const writable = await fileHandle.createWritable();

        return new OpfsWriteHandle(fileHandle, this.sessionDir, entryName, writable);
    }

    async dispose(): Promise<void> {
        if (!this.sessionDir) {
            return;
        }

        const sessionDir = this.sessionDir;
        this.sessionDir = undefined;

        try {
            const transfersDir = await this.opfsRoot.getDirectoryHandle('ironrdp-transfers');
            await transfersDir.removeEntry(this.sessionId, { recursive: true });

            // Clean up the parent directory if it is now empty.
            let hasEntries = false;
            // eslint-disable-next-line @typescript-eslint/no-unused-vars
            for await (const _ of transfersDir.values()) {
                hasEntries = true;
                break;
            }
            if (!hasEntries) {
                await this.opfsRoot.removeEntry('ironrdp-transfers');
            }
        } catch (error) {
            console.debug('OPFS session directory removal failed, falling back to per-file cleanup:', error);
            // The directory may already be gone if another tab cleaned up,
            // or the OPFS was cleared externally.  Fall back to deleting
            // individual files from the session directory handle.
            try {
                for await (const name of sessionDir.keys()) {
                    try {
                        await sessionDir.removeEntry(name);
                    } catch {
                        // Ignore per-file errors.
                    }
                }
            } catch {
                // Session dir handle may be stale; nothing more to do.
            }
        }
    }
}

/**
 * Sanitize a file name for use as an OPFS entry name.
 *
 * OPFS entry names must not contain `/` or `\`, and must not be `.` or
 * `..`.  We strip control characters, replace separators with
 * underscores, and strip leading dots.
 */
function sanitizeOpfsName(name: string): string {
    // Strip ASCII control characters (U+0000-U+001F) that could cause
    // inconsistent behavior across OPFS implementations or confuse logs.
    // eslint-disable-next-line no-control-regex
    let safe = name.replace(/[\u0000-\u001f]/g, '');

    safe = safe.replace(/[/\\]/g, '_');

    // Strip leading dots to avoid `.` / `..` collisions.
    safe = safe.replace(/^\.+/, '');

    // Ensure we always have a non-empty name.
    if (safe.length === 0) {
        safe = 'unnamed';
    }

    // OPFS entry names are typically limited to 255 bytes.  Truncate by
    // UTF-8 byte length (not JS char count) to leave room for the
    // sequence prefix added by createWriteHandle.  Non-ASCII characters
    // can be 2-4 bytes each, so a char-based limit could still exceed
    // the byte budget.
    const encoder = new TextEncoder();
    if (encoder.encode(safe).byteLength > 200) {
        while (encoder.encode(safe).byteLength > 200) {
            safe = safe.slice(0, -1);
        }
        // Ensure truncation did not leave us empty.
        if (safe.length === 0) {
            safe = 'unnamed';
        }
    }

    return safe;
}
