import type { FileStorageBackend, FileWriteHandle } from './FileStorageBackend';

/**
 * Write handle that accumulates chunks in an in-memory array and assembles
 * a {@link Blob} on {@link finalize}.
 *
 * Simple and universally supported, but peak RAM is approximately 2x the
 * file size (the chunk array plus the final Blob).  See
 * {@link OpfsStorageBackend} for a streaming alternative.
 */
class BlobWriteHandle implements FileWriteHandle {
    private chunks: Uint8Array[] = [];
    private _bytesWritten = 0;
    private finalized = false;

    get bytesWritten(): number {
        return this._bytesWritten;
    }

    async write(chunk: Uint8Array): Promise<void> {
        if (this.finalized) {
            throw new Error('BlobWriteHandle: write after finalize/abort');
        }
        this.chunks.push(chunk);
        this._bytesWritten += chunk.length;
    }

    async finalize(): Promise<Blob> {
        if (this.finalized) {
            throw new Error('BlobWriteHandle: already finalized or aborted');
        }
        this.finalized = true;
        const blob = new Blob(this.chunks);
        this.chunks = [];
        return blob;
    }

    async abort(): Promise<void> {
        if (this.finalized) {
            return;
        }
        this.finalized = true;
        this.chunks = [];
    }
}

/**
 * In-memory Blob storage backend.
 *
 * Downloads are buffered as {@link Uint8Array} chunks in a plain array and
 * assembled into a single {@link Blob} when the transfer completes.  This
 * is the universal fallback that works in every browser context.
 *
 * **Trade-offs:**
 * - Peak RAM ~2x file size (chunk array + final Blob).
 * - No persistent storage; data is lost on page unload.
 * - No setup cost; works even in non-secure contexts and private browsing.
 */
export class BlobStorageBackend implements FileStorageBackend {
    readonly name = 'blob';

    async createWriteHandle(_fileName: string, _expectedSize: number): Promise<FileWriteHandle> {
        return new BlobWriteHandle();
    }

    async dispose(): Promise<void> {
        // Nothing persistent to clean up.
    }
}
