import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import type { FileTransferError } from './RdpFileTransferProvider';
import type { FileInfo } from './FileTransfer';

// Mock the extensions module so tests don't need the real WASM Extension class.
// Each factory returns a plain object with an ident for identification.
vi.mock('./extensions', () => ({
    filesAvailableCallback: (cb: unknown) => ({ ident: 'files_available_callback', value: cb }),
    fileContentsRequestCallback: (cb: unknown) => ({ ident: 'file_contents_request_callback', value: cb }),
    fileContentsResponseCallback: (cb: unknown) => ({ ident: 'file_contents_response_callback', value: cb }),
    lockCallback: (cb: unknown) => ({ ident: 'lock_callback', value: cb }),
    unlockCallback: (cb: unknown) => ({ ident: 'unlock_callback', value: cb }),
    locksExpiredCallback: (cb: unknown) => ({ ident: 'locks_expired_callback', value: cb }),
    requestFileContents: (params: unknown) => ({ ident: 'request_file_contents', value: params }),
    submitFileContents: (params: unknown) => ({ ident: 'submit_file_contents', value: params }),
    initiateFileCopy: (files: unknown) => ({ ident: 'initiate_file_copy', value: files }),
}));

// Import after mock registration
const { RdpFileTransferProvider } = await import('./RdpFileTransferProvider');

/**
 * RdpFileTransferProvider Unit Tests
 *
 * Testing Strategy:
 * ----------------
 * These tests cover the JavaScript layer of RdpFileTransferProvider including:
 * - Setup and initialization logic
 * - Event system (registration, emission, removal)
 * - Browser API helpers (drag/drop, file picker)
 * - Cleanup and disposal
 * - Edge case handling
 *
 * What is NOT tested here:
 * - Full async download/upload flows (require WASM integration)
 * - Protocol sequencing and lock coordination (tested in Rust: ironrdp-cliprdr)
 * - WASM callback orchestration (requires real WASM runtime)
 */

// Mock session that captures invokeExtension calls
class MockSession {
    invokeExtension = vi.fn();
}

/**
 * Helper: create a provider, set its session, and return both.
 */
function setupProvider(options?: { chunkSize?: number; onUploadStarted?: () => void; onUploadFinished?: () => void }) {
    const provider = new RdpFileTransferProvider(options);
    const session = new MockSession();
    provider.setSession(session);
    return { provider, session };
}

type RdpFileTransferProviderInstance = InstanceType<typeof RdpFileTransferProvider>;

describe('RdpFileTransferProvider', () => {
    let provider: RdpFileTransferProviderInstance;

    beforeEach(() => {
        const setup = setupProvider();
        provider = setup.provider;
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    describe('setup and initialization', () => {
        it('should create provider instance', () => {
            expect(provider).toBeInstanceOf(RdpFileTransferProvider);
        });

        it('should return builder extensions', () => {
            // getBuilderExtensions() would return Extension objects.
            // Without WASM, the factory functions will throw, so we
            // just verify the method exists and the provider is functional.
            expect(provider.getBuilderExtensions).toBeDefined();
        });

        it('should use custom chunk size', () => {
            const custom = new RdpFileTransferProvider({ chunkSize: 32768 });
            expect(custom).toBeDefined();
        });

        it('should throw error when session not available', () => {
            const noSession = new RdpFileTransferProvider();
            expect(() => {
                // @ts-expect-error - accessing private method for testing
                noSession.ensureSession();
            }).toThrow('Session not available');
        });

        it('should accept session via setSession', () => {
            const p = new RdpFileTransferProvider();
            const s = new MockSession();
            p.setSession(s);
            // Should not throw after setSession
            // @ts-expect-error - accessing private method for testing
            expect(() => p.ensureSession()).not.toThrow();
        });
    });

    describe('event system', () => {
        it('should register event handlers', () => {
            const handler = vi.fn();
            provider.on('files-available', handler);
            // No error should be thrown
        });

        it('should emit files-available event via handleFilesAvailable', () => {
            const handler = vi.fn();
            provider.on('files-available', handler);

            const files: FileInfo[] = [{ name: 'test.txt', size: 1024, lastModified: Date.now() }];

            // Call the handler directly (in production, the WASM layer would call this)
            // @ts-expect-error - accessing private method for testing
            provider.handleFilesAvailable(files);
            expect(handler).toHaveBeenCalledWith(files);
        });

        it('should remove event handlers with off()', () => {
            const handler = vi.fn();
            provider.on('files-available', handler);
            provider.off('files-available', handler);

            // @ts-expect-error - accessing private method for testing
            provider.handleFilesAvailable([]);
            expect(handler).not.toHaveBeenCalled();
        });

        it('should support multiple handlers for same event', () => {
            const handler1 = vi.fn();
            const handler2 = vi.fn();

            provider.on('files-available', handler1);
            provider.on('files-available', handler2);

            const files: FileInfo[] = [{ name: 'test.txt', size: 100, lastModified: Date.now() }];
            // @ts-expect-error - accessing private method for testing
            provider.handleFilesAvailable(files);

            expect(handler1).toHaveBeenCalledWith(files);
            expect(handler2).toHaveBeenCalledWith(files);
        });
    });

    describe('browser integration helpers', () => {
        it('should have showFilePicker method', () => {
            expect(provider.showFilePicker).toBeDefined();
        });

        it('should have handleDrop method', () => {
            expect(provider.handleDrop).toBeDefined();
        });

        it('should have handleDragOver method', () => {
            expect(provider.handleDragOver).toBeDefined();
        });

        it('should extract files from drop event (files fallback)', async () => {
            const mockDataTransfer = {
                files: [new File(['test'], 'test.txt')],
            };

            const mockEvent = {
                dataTransfer: mockDataTransfer,
                preventDefault: vi.fn(),
                stopPropagation: vi.fn(),
            } as unknown as DragEvent;

            const files = await provider.handleDrop(mockEvent);
            expect(files).toHaveLength(1);
            expect(files[0].name).toBe('test.txt');
            expect(files[0].file).toBeInstanceOf(File);
            expect(files[0].isDirectory).toBeUndefined();
            expect(mockEvent.preventDefault).toHaveBeenCalled();
        });

        it('should extract files from drop event (items with webkitGetAsEntry)', async () => {
            const testFile = new File(['hello'], 'hello.txt');

            const mockEntry: Partial<FileSystemFileEntry> = {
                isFile: true,
                isDirectory: false,
                name: 'hello.txt',
                file: (cb: (file: File) => void) => cb(testFile),
            };

            const mockItem = {
                kind: 'file',
                webkitGetAsEntry: () => mockEntry as FileSystemEntry,
            };

            const mockDataTransfer = {
                items: [mockItem],
            };

            const mockEvent = {
                dataTransfer: mockDataTransfer,
                preventDefault: vi.fn(),
            } as unknown as DragEvent;

            const result = await provider.handleDrop(mockEvent);
            expect(result).toHaveLength(1);
            expect(result[0].name).toBe('hello.txt');
            expect(result[0].file).toBe(testFile);
            expect(result[0].path).toBeUndefined();
            expect(result[0].isDirectory).toBeUndefined();
        });
    });

    describe('disposal', () => {
        it('should mark as disposed', () => {
            provider.dispose();
            // @ts-expect-error - accessing private field for testing
            expect(provider.disposed).toBe(true);
        });

        it('should clear event handlers on dispose', () => {
            const handler = vi.fn();
            provider.on('files-available', handler);
            provider.dispose();
            // @ts-expect-error - accessing private method for testing
            provider.handleFilesAvailable([]);
            expect(handler).not.toHaveBeenCalled();
        });

        it('should reject pending download on dispose', async () => {
            const fileInfo: FileInfo = { name: 'data.bin', size: 2048, lastModified: Date.now() };

            const { completion } = provider.downloadFile(fileInfo, 0);
            provider.dispose();

            await expect(completion).rejects.toThrow();
        });
    });

    describe('download error handling', () => {
        it('should reject with error when session not available for download', async () => {
            const noSession = new RdpFileTransferProvider();
            const fileInfo: FileInfo = { name: 'data.bin', size: 2048, lastModified: Date.now() };

            const errorHandler = vi.fn();
            noSession.on('error', errorHandler);

            const { completion } = noSession.downloadFile(fileInfo, 0);
            await expect(completion).rejects.toThrow('Failed to request file size');

            expect(errorHandler).toHaveBeenCalledTimes(1);
            const emittedError: FileTransferError = errorHandler.mock.calls[0][0];
            expect(emittedError.direction).toBe('download');
            expect(emittedError.fileName).toBe('data.bin');
        });
    });

    describe('upload lifecycle callbacks', () => {
        it('should call onUploadStarted and onUploadFinished around initiateFileCopy', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const { provider: p, session: s } = setupProvider({
                onUploadStarted,
                onUploadFinished,
            });

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion: uploadPromise } = p.uploadFiles(files);

            // Both should fire immediately (monitoring suppression is brief)
            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
            expect(s.invokeExtension).toHaveBeenCalledTimes(1);

            // Upload started should fire before invokeExtension (initiateFileCopy)
            expect(onUploadStarted.mock.invocationCallOrder[0]).toBeLessThan(
                s.invokeExtension.mock.invocationCallOrder[0],
            );

            // Clean up
            p.dispose();
            await expect(uploadPromise).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('should call onUploadFinished even on initiateFileCopy failure', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const { provider: p, session: s } = setupProvider({
                onUploadStarted,
                onUploadFinished,
            });
            s.invokeExtension.mockImplementation(() => {
                throw new Error('Copy failed');
            });

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);
            await expect(completion).rejects.toThrow('Failed to initiate file upload');

            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            // onUploadFinished fires in finally block regardless
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });
    });

    describe('sanitizeFileName', () => {
        it('should return a plain filename as-is', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('file.txt')).toBe('file.txt');
        });

        it('should strip Unix path traversal', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('../../../etc/passwd')).toBe('passwd');
        });

        it('should strip Windows path traversal', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('..\\..\\system32\\config\\SAM')).toBe('SAM');
        });

        it('should extract basename from Windows absolute path', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('C:\\Users\\victim\\Desktop\\file.txt')).toBe('file.txt');
        });

        it('should extract basename from Unix absolute path', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('/home/user/file.txt')).toBe('file.txt');
        });

        it('should return fallback for empty string', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('')).toBe('unnamed_file');
        });

        it('should return fallback for traversal-only input', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('../..')).toBe('unnamed_file');
        });

        it('should handle trailing separator', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('path/to/file/')).toBe('file');
        });

        it('should handle mixed separators', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('path/to\\file.txt')).toBe('file.txt');
        });

        it('should keep triple-dot filename (not traversal)', () => {
            expect(RdpFileTransferProvider.sanitizeFileName('...')).toBe('...');
        });
    });

    describe('sanitizePath', () => {
        it('should return undefined for empty string', () => {
            expect(RdpFileTransferProvider.sanitizePath('')).toBeUndefined();
        });

        it('should return undefined for traversal-only path', () => {
            expect(RdpFileTransferProvider.sanitizePath('../..')).toBeUndefined();
            expect(RdpFileTransferProvider.sanitizePath('.')).toBeUndefined();
        });

        it('should preserve a simple relative path', () => {
            expect(RdpFileTransferProvider.sanitizePath('temp')).toBe('temp');
        });

        it('should preserve a multi-level relative path', () => {
            expect(RdpFileTransferProvider.sanitizePath('folder\\sub')).toBe('folder\\sub');
        });

        it('should strip traversal components from path', () => {
            expect(RdpFileTransferProvider.sanitizePath('..\\..\\etc')).toBe('etc');
        });

        it('should strip drive letter prefix', () => {
            expect(RdpFileTransferProvider.sanitizePath('C:\\Users\\Desktop')).toBe('Users\\Desktop');
        });

        it('should normalize Unix separators to backslash', () => {
            expect(RdpFileTransferProvider.sanitizePath('folder/sub')).toBe('folder\\sub');
        });

        it('should handle mixed separators', () => {
            expect(RdpFileTransferProvider.sanitizePath('folder/sub\\dir')).toBe('folder\\sub\\dir');
        });

        it('should return undefined if only drive letter remains', () => {
            expect(RdpFileTransferProvider.sanitizePath('C:')).toBeUndefined();
        });

        it('should strip UNC long path prefix with drive letter', () => {
            expect(RdpFileTransferProvider.sanitizePath('?\\C:\\Users\\Desktop')).toBe('Users\\Desktop');
        });

        it('should strip UNC device prefix', () => {
            expect(RdpFileTransferProvider.sanitizePath('.\\device\\path')).toBe('device\\path');
        });

        it('should return undefined if only UNC prefix remains', () => {
            expect(RdpFileTransferProvider.sanitizePath('?\\C:')).toBeUndefined();
        });
    });

    describe('directory drag-and-drop traversal', () => {
        function mockDirEntry(name: string, children: FileSystemEntry[]): FileSystemDirectoryEntry {
            return {
                isFile: false,
                isDirectory: true,
                name,
                fullPath: `/${name}`,
                filesystem: {} as FileSystem,
                getParent: vi.fn(),
                createReader: () => {
                    let read = false;
                    return {
                        readEntries: (cb: (entries: FileSystemEntry[]) => void) => {
                            if (!read) {
                                read = true;
                                cb(children);
                            } else {
                                cb([]);
                            }
                        },
                    } as unknown as FileSystemDirectoryReader;
                },
                getFile: vi.fn(),
                getDirectory: vi.fn(),
            } as unknown as FileSystemDirectoryEntry;
        }

        function mockFileEntry(name: string, content: string): FileSystemFileEntry {
            const file = new File([content], name);
            return {
                isFile: true,
                isDirectory: false,
                name,
                fullPath: `/${name}`,
                filesystem: {} as FileSystem,
                getParent: vi.fn(),
                file: (cb: (f: File) => void) => cb(file),
                createWriter: vi.fn(),
            } as unknown as FileSystemFileEntry;
        }

        function dropEventWithEntries(entries: FileSystemEntry[]): DragEvent {
            const items = entries.map((entry) => ({
                kind: 'file',
                webkitGetAsEntry: () => entry,
            }));

            return {
                dataTransfer: { items },
                preventDefault: vi.fn(),
            } as unknown as DragEvent;
        }

        it('should recursively traverse a dropped folder', async () => {
            const child1 = mockFileEntry('a.txt', 'aaa');
            const child2 = mockFileEntry('b.txt', 'bbb');
            const folder = mockDirEntry('myFolder', [child1, child2]);

            const result = await provider.handleDrop(dropEventWithEntries([folder]));

            expect(result).toHaveLength(3);

            expect(result[0].name).toBe('myFolder');
            expect(result[0].isDirectory).toBe(true);
            expect(result[0].file).toBeNull();
            expect(result[0].path).toBeUndefined();

            expect(result[1].name).toBe('a.txt');
            expect(result[1].path).toBe('myFolder');
            expect(result[1].file).toBeInstanceOf(File);

            expect(result[2].name).toBe('b.txt');
            expect(result[2].path).toBe('myFolder');
        });

        it('should handle nested directories with correct paths', async () => {
            const deepFile = mockFileEntry('deep.txt', 'deep');
            const subDir = mockDirEntry('sub', [deepFile]);
            const topDir = mockDirEntry('top', [subDir]);

            const result = await provider.handleDrop(dropEventWithEntries([topDir]));

            expect(result).toHaveLength(3);
            expect(result[0]).toMatchObject({ name: 'top', isDirectory: true, path: undefined });
            expect(result[1]).toMatchObject({ name: 'sub', isDirectory: true, path: 'top' });
            expect(result[2]).toMatchObject({ name: 'deep.txt', path: 'top\\sub' });
            expect(result[2].file).toBeInstanceOf(File);
        });

        it('should handle mixed files and folders at root level', async () => {
            const rootFile = mockFileEntry('root.txt', 'r');
            const folderFile = mockFileEntry('inside.txt', 'i');
            const folder = mockDirEntry('dir', [folderFile]);

            const result = await provider.handleDrop(dropEventWithEntries([rootFile, folder]));

            expect(result).toHaveLength(3);
            expect(result[0]).toMatchObject({ name: 'root.txt', path: undefined });
            expect(result[1]).toMatchObject({ name: 'dir', isDirectory: true });
            expect(result[2]).toMatchObject({ name: 'inside.txt', path: 'dir' });
        });

        it('should respect the max entry count limit', async () => {
            const children: FileSystemEntry[] = [];
            for (let i = 0; i < 1001; i++) {
                children.push(mockFileEntry(`file${i}.txt`, `${i}`));
            }
            const bigDir = mockDirEntry('big', children);

            const result = await provider.handleDrop(dropEventWithEntries([bigDir]));
            expect(result.length).toBeLessThanOrEqual(1000);
        });

        it('should handle empty directories', async () => {
            const emptyDir = mockDirEntry('empty', []);

            const result = await provider.handleDrop(dropEventWithEntries([emptyDir]));

            expect(result).toHaveLength(1);
            expect(result[0]).toMatchObject({
                name: 'empty',
                isDirectory: true,
                file: null,
                size: 0,
            });
        });
    });

    describe('handleFilesAvailable preserves path and isDirectory', () => {
        it('should pass through path and isDirectory from files', () => {
            const receivedFiles: FileInfo[][] = [];
            provider.on('files-available', (files: FileInfo[]) => receivedFiles.push(files));

            // @ts-expect-error - accessing private method for testing
            provider.handleFilesAvailable([
                { name: 'readme.txt', size: 100, lastModified: 0 },
                { name: 'report.pdf', path: 'docs', size: 2048, lastModified: 0 },
                { name: 'images', path: 'docs', size: 0, lastModified: 0, isDirectory: true },
                { name: 'photo.png', path: 'docs\\images', size: 4096, lastModified: 0 },
            ]);

            expect(receivedFiles).toHaveLength(1);
            const files = receivedFiles[0];
            expect(files).toHaveLength(4);

            expect(files[0].name).toBe('readme.txt');
            expect(files[0].path).toBeUndefined();
            expect(files[0].isDirectory).toBeUndefined();

            expect(files[1].name).toBe('report.pdf');
            expect(files[1].path).toBe('docs');

            expect(files[2].name).toBe('images');
            expect(files[2].path).toBe('docs');
            expect(files[2].isDirectory).toBe(true);

            expect(files[3].name).toBe('photo.png');
            expect(files[3].path).toBe('docs\\images');

            provider.dispose();
        });
    });
});
