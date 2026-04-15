import type { FileStorageBackend } from './FileStorageBackend';
import { BlobStorageBackend } from './BlobStorageBackend';
import { OpfsStorageBackend } from './OpfsStorageBackend';

/**
 * Storage backend preference for downloads.
 *
 * - `'auto'` - detect the best available backend (OPFS with Blob
 *   fallback).  OPFS reduces peak download RAM from ~2x file size to
 *   ~chunk size.
 * - `'blob'` - force in-memory Blob storage regardless of OPFS
 *   availability.
 */
export type StorageBackendPreference = 'auto' | 'blob';

/**
 * Detect the best available storage backend for the current browser
 * context.
 *
 * When `preference` is `'auto'` (default), the function probes for OPFS
 * support with a full round-trip smoke test.  If OPFS is available, an
 * {@link OpfsStorageBackend} is returned; otherwise a
 * {@link BlobStorageBackend} is used as the universal fallback.
 *
 * When `preference` is `'blob'`, OPFS detection is skipped entirely and
 * the Blob backend is returned immediately.
 *
 * @param preference - `'auto'` to detect, `'blob'` to force in-memory.
 * @param sessionId  - Optional session identifier used as the OPFS
 *                    subdirectory name.  When omitted a unique ID is
 *                    generated from the current timestamp.
 * @returns The selected backend, ready to use.
 */
export async function detectStorageBackend(
    preference: StorageBackendPreference = 'auto',
    sessionId?: string,
): Promise<FileStorageBackend> {
    if (preference === 'blob') {
        return new BlobStorageBackend();
    }

    // Attempt OPFS detection.
    if (typeof globalThis.navigator?.storage?.getDirectory === 'function') {
        try {
            const opfsRoot = await navigator.storage.getDirectory();

            if (await OpfsStorageBackend.probe(opfsRoot)) {
                return OpfsStorageBackend.create(opfsRoot, sessionId);
            }

            // Probe failed: the OPFS API exists but is not functional
            // (e.g., createWritable() throws in some browser modes).
            console.debug('OPFS probe failed (createWritable not functional), falling back to blob storage');
        } catch (error) {
            // getDirectory() itself threw (e.g., SecurityError in some
            // private browsing modes).  Fall through to Blob.
            console.debug('OPFS unavailable, falling back to blob storage:', error);
        }
    }

    return new BlobStorageBackend();
}
