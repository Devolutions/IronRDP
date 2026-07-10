import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { BlobStorageBackend } from './BlobStorageBackend';
import { OpfsStorageBackend } from './OpfsStorageBackend';
import { detectStorageBackend } from './detect';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function chunk(bytes: number[]): Uint8Array {
    return new Uint8Array(bytes);
}

async function blobToBytes(blob: Blob): Promise<Uint8Array> {
    // jsdom Blob.arrayBuffer() may not be available or may behave
    // inconsistently.  Use FileReader which jsdom supports reliably.
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer));
        reader.onerror = () => reject(reader.error);
        reader.readAsArrayBuffer(blob);
    });
}

// ---------------------------------------------------------------------------
// Minimal in-memory mock of the OPFS directory/file handle API.
// Shared across OpfsStorageBackend and detectStorageBackend test suites.
// ---------------------------------------------------------------------------

interface MockEntry {
    kind: 'file' | 'directory';
    name: string;
    children?: Map<string, MockEntry>;
    content?: Uint8Array[];
    writable?: MockWritable;
}

class MockWritable {
    closed = false;
    aborted = false;
    private target: MockEntry;

    constructor(target: MockEntry) {
        this.target = target;
        // Reset content on each new writable (mirrors real createWritable)
        this.target.content = [];
    }

    async write(data: Uint8Array): Promise<void> {
        if (this.closed || this.aborted) throw new Error('stream closed');
        this.target.content!.push(new Uint8Array(data));
    }

    async close(): Promise<void> {
        this.closed = true;
    }

    async abort(): Promise<void> {
        this.aborted = true;
    }
}

function createMockFileHandle(entry: MockEntry): FileSystemFileHandle {
    return {
        kind: 'file' as const,
        name: entry.name,
        isSameEntry: vi.fn(),
        async getFile() {
            const parts = entry.content ?? [];
            return new Blob(parts);
        },
        async createWritable() {
            const w = new MockWritable(entry);
            entry.writable = w;
            return w as unknown as FileSystemWritableFileStream;
        },
        async createSyncAccessHandle() {
            throw new Error('not implemented');
        },
    } as unknown as FileSystemFileHandle;
}

function createMockDirectoryHandle(name: string, children?: Map<string, MockEntry>): FileSystemDirectoryHandle {
    const entries: Map<string, MockEntry> = children ?? new Map();

    function makeValuesIterator(): AsyncIterableIterator<FileSystemHandle> {
        const values = [...entries.values()];
        let index = 0;
        return {
            [Symbol.asyncIterator]() {
                return this;
            },
            async next() {
                if (index < values.length) {
                    return { value: values[index++] as unknown as FileSystemHandle, done: false as const };
                }
                return { value: undefined, done: true as const };
            },
        };
    }

    function makeKeysIterator(): AsyncIterableIterator<string> {
        const keys = [...entries.keys()];
        let index = 0;
        return {
            [Symbol.asyncIterator]() {
                return this;
            },
            async next() {
                if (index < keys.length) {
                    return { value: keys[index++], done: false as const };
                }
                return { value: undefined, done: true as const };
            },
        };
    }

    return {
        kind: 'directory' as const,
        name,
        isSameEntry: vi.fn(),
        async getFileHandle(fileName: string, options?: FileSystemGetFileOptions) {
            let entry = entries.get(fileName);
            if (entry === undefined && options?.create === true) {
                entry = { kind: 'file', name: fileName, content: [] };
                entries.set(fileName, entry);
            }
            if (entry === undefined) throw new DOMException('NotFoundError');
            return createMockFileHandle(entry);
        },
        async getDirectoryHandle(dirName: string, options?: FileSystemGetDirectoryOptions) {
            let entry = entries.get(dirName);
            if (entry === undefined && options?.create === true) {
                entry = { kind: 'directory', name: dirName, children: new Map() };
                entries.set(dirName, entry);
            }
            if (entry === undefined) throw new DOMException('NotFoundError');
            return createMockDirectoryHandle(dirName, entry.children);
        },
        async removeEntry(entryName: string, _options?: FileSystemRemoveOptions) {
            if (!entries.has(entryName)) throw new DOMException('NotFoundError');
            entries.delete(entryName);
        },
        async resolve(_child: FileSystemHandle) {
            return null;
        },
        values: makeValuesIterator,
        keys: makeKeysIterator,
        entries() {
            return makeValuesIterator() as unknown as AsyncIterableIterator<[string, FileSystemHandle]>;
        },
        [Symbol.asyncIterator]() {
            return makeValuesIterator() as unknown as AsyncIterableIterator<[string, FileSystemHandle]>;
        },
    } as unknown as FileSystemDirectoryHandle;
}

// ---------------------------------------------------------------------------
// BlobStorageBackend
// ---------------------------------------------------------------------------

describe('BlobStorageBackend', () => {
    let backend: BlobStorageBackend;

    beforeEach(() => {
        backend = new BlobStorageBackend();
    });

    it('has name "blob"', () => {
        expect(backend.name).toBe('blob');
    });

    it('write then finalize produces correct Blob', async () => {
        const handle = await backend.createWriteHandle('test.bin', 6);

        await handle.write(chunk([1, 2, 3]));
        expect(handle.bytesWritten).toBe(3);

        await handle.write(chunk([4, 5, 6]));
        expect(handle.bytesWritten).toBe(6);

        const blob = await handle.finalize();
        expect(blob).toBeInstanceOf(Blob);
        expect(blob.size).toBe(6);

        const data = await blobToBytes(blob);
        expect(data).toEqual(new Uint8Array([1, 2, 3, 4, 5, 6]));
    });

    it('finalize on empty handle returns empty Blob', async () => {
        const handle = await backend.createWriteHandle('empty.bin', 0);
        const blob = await handle.finalize();
        expect(blob.size).toBe(0);
        expect(handle.bytesWritten).toBe(0);
    });

    it('abort clears state and prevents reuse', async () => {
        const handle = await backend.createWriteHandle('test.bin', 3);
        await handle.write(chunk([1, 2, 3]));
        await handle.abort();

        // Write after abort throws
        await expect(handle.write(chunk([4]))).rejects.toThrow(/finalize|abort/);
        // Finalize after abort throws
        await expect(handle.finalize()).rejects.toThrow(/finalize|abort/);
    });

    it('double abort is safe', async () => {
        const handle = await backend.createWriteHandle('test.bin', 0);
        await handle.abort();
        await expect(handle.abort()).resolves.toBeUndefined();
    });

    it('write after finalize throws', async () => {
        const handle = await backend.createWriteHandle('test.bin', 0);
        await handle.finalize();
        await expect(handle.write(chunk([1]))).rejects.toThrow(/finalize|abort/);
    });

    it('double finalize throws', async () => {
        const handle = await backend.createWriteHandle('test.bin', 0);
        await handle.finalize();
        await expect(handle.finalize()).rejects.toThrow(/finalize|abort/);
    });

    it('dispose is a no-op', async () => {
        await expect(backend.dispose()).resolves.toBeUndefined();
    });

    it('multiple concurrent handles are independent', async () => {
        const h1 = await backend.createWriteHandle('a.bin', 2);
        const h2 = await backend.createWriteHandle('b.bin', 2);

        await h1.write(chunk([10, 20]));
        await h2.write(chunk([30, 40]));

        const b1 = await h1.finalize();
        const b2 = await h2.finalize();

        expect(await blobToBytes(b1)).toEqual(new Uint8Array([10, 20]));
        expect(await blobToBytes(b2)).toEqual(new Uint8Array([30, 40]));
    });
});

// ---------------------------------------------------------------------------
// OpfsStorageBackend - mocked OPFS
// ---------------------------------------------------------------------------

describe('OpfsStorageBackend', () => {
    let mockOpfsRoot: FileSystemDirectoryHandle;

    beforeEach(() => {
        mockOpfsRoot = createMockDirectoryHandle('');
    });

    describe('probe', () => {
        it('returns true when OPFS works', async () => {
            expect(await OpfsStorageBackend.probe(mockOpfsRoot)).toBe(true);
        });

        it('returns false when createWritable throws', async () => {
            const broken = {
                ...mockOpfsRoot,
                async getFileHandle() {
                    return {
                        async createWritable() {
                            throw new Error('SecurityError');
                        },
                    } as unknown as FileSystemFileHandle;
                },
            } as unknown as FileSystemDirectoryHandle;

            expect(await OpfsStorageBackend.probe(broken)).toBe(false);
        });
    });

    describe('lifecycle', () => {
        it('creates session directory on construction', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'test-session');
            expect(backend.name).toBe('opfs');

            // Verify the session directory was created
            const transfersDir = await mockOpfsRoot.getDirectoryHandle('ironrdp-transfers');
            const sessionDir = await transfersDir.getDirectoryHandle('test-session');
            expect(sessionDir.name).toBe('test-session');

            await backend.dispose();
        });

        it('write/finalize produces a Blob', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-1');
            const handle = await backend.createWriteHandle('hello.bin', 4);

            await handle.write(chunk([10, 20]));
            expect(handle.bytesWritten).toBe(2);

            await handle.write(chunk([30, 40]));
            expect(handle.bytesWritten).toBe(4);

            const blob = await handle.finalize();
            expect(blob.size).toBe(4);

            const data = await blobToBytes(blob);
            expect(data).toEqual(new Uint8Array([10, 20, 30, 40]));

            await backend.dispose();
        });

        it('abort removes the temp file', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-2');
            const handle = await backend.createWriteHandle('abort-me.bin', 4);
            await handle.write(chunk([1, 2, 3, 4]));
            await handle.abort();

            // Write after abort should throw
            await expect(handle.write(chunk([5]))).rejects.toThrow();

            await backend.dispose();
        });

        it('double abort is safe', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-3');
            const handle = await backend.createWriteHandle('test.bin', 0);
            await handle.abort();
            await expect(handle.abort()).resolves.toBeUndefined();
            await backend.dispose();
        });

        it('write after finalize throws', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-waf');
            const handle = await backend.createWriteHandle('test.bin', 0);
            await handle.finalize();
            await expect(handle.write(chunk([1]))).rejects.toThrow(/finalize|abort/);
            await backend.dispose();
        });

        it('double finalize throws', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-df');
            const handle = await backend.createWriteHandle('test.bin', 0);
            await handle.finalize();
            await expect(handle.finalize()).rejects.toThrow(/finalize|abort/);
            await backend.dispose();
        });

        it('dispose cleans up session directory', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-cleanup');
            await backend.createWriteHandle('file1.bin', 0);
            await backend.dispose();

            // When the session was the only child, dispose also removes the
            // parent `ironrdp-transfers` directory.  Either the parent or
            // the session subdir being gone confirms cleanup succeeded.
            let sessionDirExists = true;
            try {
                const transfersDir = await mockOpfsRoot.getDirectoryHandle('ironrdp-transfers');
                await transfersDir.getDirectoryHandle('sess-cleanup');
            } catch {
                sessionDirExists = false;
            }
            expect(sessionDirExists).toBe(false);
        });

        it('double dispose is safe', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-double');
            await backend.dispose();
            await expect(backend.dispose()).resolves.toBeUndefined();
        });

        it('createWriteHandle after dispose throws', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-after');
            await backend.dispose();
            await expect(backend.createWriteHandle('nope.bin', 0)).rejects.toThrow(/disposed/);
        });

        it('sanitizes file names for OPFS entries', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-sanitize');

            // Collect entry names created in the session directory.
            const transfersDir = await mockOpfsRoot.getDirectoryHandle('ironrdp-transfers');
            const sessionDir = await transfersDir.getDirectoryHandle('sess-sanitize');
            async function entryNames(): Promise<string[]> {
                const names: string[] = [];
                for await (const name of sessionDir.keys()) names.push(name);
                return names;
            }

            // Path traversal: slashes become underscores, dots are interior
            // (leading-dot stripping only applies after separator replacement).
            const h1 = await backend.createWriteHandle('../../etc/passwd', 3);
            await h1.write(chunk([1, 2, 3]));
            await h1.finalize();
            expect(await entryNames()).toEqual(['0-_.._etc_passwd']);

            // Bare ".." becomes empty after stripping, falls back to "unnamed".
            const h2 = await backend.createWriteHandle('..', 1);
            await h2.write(chunk([1]));
            await h2.finalize();
            expect(await entryNames()).toContain('1-unnamed');

            // Leading dots stripped, rest preserved.
            const h3 = await backend.createWriteHandle('.hidden', 1);
            await h3.write(chunk([1]));
            await h3.finalize();
            expect(await entryNames()).toContain('2-hidden');

            // Control characters (null bytes, tabs, newlines) are stripped.
            const h4 = await backend.createWriteHandle('foo\x00bar\tbaz\n.txt', 1);
            await h4.write(chunk([1]));
            await h4.finalize();
            expect(await entryNames()).toContain('3-foobarbaz.txt');

            // Multi-byte UTF-8 names are truncated by byte length, not char count.
            // Each emoji is 4 UTF-8 bytes; 51 emojis = 204 bytes > 200 byte limit.
            const longEmoji = '\u{1F600}'.repeat(51); // 51 x 4 = 204 bytes
            const h5 = await backend.createWriteHandle(longEmoji, 1);
            await h5.write(chunk([1]));
            await h5.finalize();
            const names = await entryNames();
            const emojiEntry = names.find((n) => n.startsWith('4-'));
            expect(emojiEntry).toBeDefined();
            // Should be truncated to at most 200 UTF-8 bytes (50 emojis = 200 bytes).
            const encoder = new TextEncoder();
            const sanitized = emojiEntry!.slice(2); // strip "4-" prefix
            expect(encoder.encode(sanitized).byteLength).toBeLessThanOrEqual(200);
            expect(encoder.encode(sanitized).byteLength).toBeGreaterThan(0);

            await backend.dispose();
        });

        it('handles concurrent writes to different files', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-concurrent');

            const h1 = await backend.createWriteHandle('a.bin', 2);
            const h2 = await backend.createWriteHandle('b.bin', 2);

            await h1.write(chunk([1, 2]));
            await h2.write(chunk([3, 4]));

            const b1 = await h1.finalize();
            const b2 = await h2.finalize();

            expect(await blobToBytes(b1)).toEqual(new Uint8Array([1, 2]));
            expect(await blobToBytes(b2)).toEqual(new Uint8Array([3, 4]));

            await backend.dispose();
        });

        it('generates unique entry names for duplicate file names', async () => {
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, 'sess-dup');

            // Two downloads of the same file name should not collide
            const h1 = await backend.createWriteHandle('same.bin', 1);
            const h2 = await backend.createWriteHandle('same.bin', 1);

            await h1.write(chunk([10]));
            await h2.write(chunk([20]));

            const b1 = await h1.finalize();
            const b2 = await h2.finalize();

            // They should be independent
            expect(await blobToBytes(b1)).toEqual(new Uint8Array([10]));
            expect(await blobToBytes(b2)).toEqual(new Uint8Array([20]));

            await backend.dispose();
        });
    });

    describe('stale session cleanup', () => {
        it('removes session directories older than 24 hours on create', async () => {
            // Pre-populate ironrdp-transfers/ with stale and fresh entries.
            const transfersDir = await mockOpfsRoot.getDirectoryHandle('ironrdp-transfers', { create: true });

            const staleTimestamp = Date.now() - 25 * 60 * 60 * 1000; // 25 hours ago
            const freshTimestamp = Date.now() - 1 * 60 * 60 * 1000; // 1 hour ago
            const staleId = `s-${staleTimestamp}-abc123`;
            const freshId = `s-${freshTimestamp}-def456`;

            await transfersDir.getDirectoryHandle(staleId, { create: true });
            await transfersDir.getDirectoryHandle(freshId, { create: true });

            // Creating a new backend triggers cleanupStale (fire-and-forget).
            const backend = await OpfsStorageBackend.create(mockOpfsRoot, `s-${Date.now()}-new000`);
            await new Promise((r) => setTimeout(r, 10));

            // The stale directory should have been removed.
            let staleExists = true;
            try {
                await transfersDir.getDirectoryHandle(staleId);
            } catch {
                staleExists = false;
            }
            expect(staleExists).toBe(false);

            // The fresh directory should still exist.
            const freshDir = await transfersDir.getDirectoryHandle(freshId);
            expect(freshDir.name).toBe(freshId);

            await backend.dispose();
        });

        it('skips directories with unparsable names', async () => {
            const transfersDir = await mockOpfsRoot.getDirectoryHandle('ironrdp-transfers', { create: true });

            // Non-matching names should be left alone.
            await transfersDir.getDirectoryHandle('custom-dir', { create: true });

            const backend = await OpfsStorageBackend.create(mockOpfsRoot, `s-${Date.now()}-test00`);
            await new Promise((r) => setTimeout(r, 10));

            const customDir = await transfersDir.getDirectoryHandle('custom-dir');
            expect(customDir.name).toBe('custom-dir');

            await backend.dispose();
        });
    });
});

// ---------------------------------------------------------------------------
// detectStorageBackend
// ---------------------------------------------------------------------------

describe('detectStorageBackend', () => {
    const originalNavigator = globalThis.navigator;

    afterEach(() => {
        // Restore navigator after each test
        Object.defineProperty(globalThis, 'navigator', {
            value: originalNavigator,
            writable: true,
            configurable: true,
        });
    });

    it('returns BlobStorageBackend when preference is "blob"', async () => {
        const backend = await detectStorageBackend('blob');
        expect(backend.name).toBe('blob');
    });

    it('returns BlobStorageBackend when navigator.storage is unavailable', async () => {
        Object.defineProperty(globalThis, 'navigator', {
            value: {},
            writable: true,
            configurable: true,
        });

        const backend = await detectStorageBackend('auto');
        expect(backend.name).toBe('blob');
    });

    it('returns BlobStorageBackend when getDirectory throws', async () => {
        Object.defineProperty(globalThis, 'navigator', {
            value: {
                storage: {
                    getDirectory: () => Promise.reject(new Error('SecurityError')),
                },
            },
            writable: true,
            configurable: true,
        });

        const backend = await detectStorageBackend('auto');
        expect(backend.name).toBe('blob');
    });

    it('defaults to auto when no preference given', async () => {
        Object.defineProperty(globalThis, 'navigator', {
            value: {},
            writable: true,
            configurable: true,
        });

        const backend = await detectStorageBackend();
        expect(backend.name).toBe('blob');
    });

    it('returns BlobStorageBackend when OPFS probe fails', async () => {
        // getDirectory() succeeds, but createWritable() throws inside probe().
        const brokenRoot = {
            ...createMockDirectoryHandle(''),
            async getFileHandle() {
                return {
                    async createWritable() {
                        throw new Error('SecurityError');
                    },
                } as unknown as FileSystemFileHandle;
            },
        } as unknown as FileSystemDirectoryHandle;

        Object.defineProperty(globalThis, 'navigator', {
            value: {
                storage: {
                    getDirectory: () => Promise.resolve(brokenRoot),
                },
            },
            writable: true,
            configurable: true,
        });

        const backend = await detectStorageBackend('auto');
        expect(backend.name).toBe('blob');
    });

    it('returns OpfsStorageBackend when OPFS is available', async () => {
        const mockRoot = createMockDirectoryHandle('');
        Object.defineProperty(globalThis, 'navigator', {
            value: {
                storage: {
                    getDirectory: () => Promise.resolve(mockRoot),
                },
            },
            writable: true,
            configurable: true,
        });

        const backend = await detectStorageBackend('auto');
        expect(backend.name).toBe('opfs');
        await backend.dispose();
    });
});
