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
    formatListResponseCallback: (cb: unknown) => ({ ident: 'format_list_response_callback', value: cb }),
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
        it('suppresses monitoring on advertise and defers the resume past the wire send', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const { provider: p, session: s } = setupProvider({
                onUploadStarted,
                onUploadFinished,
            });

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion: uploadPromise } = p.uploadFiles(files);

            // Monitoring is suppressed before the FormatList goes on the wire...
            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            expect(s.invokeExtension).toHaveBeenCalledTimes(1);
            expect(onUploadStarted.mock.invocationCallOrder[0]).toBeLessThan(
                s.invokeExtension.mock.invocationCallOrder[0],
            );
            // ...but the resume is now DEFERRED (held until the paste is pulled or we
            // give up), so the 100ms monitor poll cannot clobber the file FormatList.
            expect(onUploadFinished).not.toHaveBeenCalled();

            // Dispose resumes monitoring exactly once and rejects the pending upload.
            p.dispose();
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
            await expect(uploadPromise).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('resumes monitoring on the first FileContentsRequest (remote pulled the files)', async () => {
            const onUploadFinished = vi.fn();
            const { provider: p } = setupProvider({ onUploadFinished });

            const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);
            expect(onUploadFinished).not.toHaveBeenCalled();

            // Remote requests file size for index 0 -> paste was pulled -> resume.
            // @ts-expect-error - exercising the private callback the WASM layer drives
            p.handleFileContentsRequest({ streamId: 1, index: 0, flags: 1, position: 0, size: 8 });
            expect(onUploadFinished).toHaveBeenCalledTimes(1);

            p.dispose();
            // Already resumed on the pull, so dispose does not resume again.
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
            await expect(completion).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('fails the upload and resumes monitoring if the remote never pulls (watchdog)', async () => {
            vi.useFakeTimers();
            try {
                const onUploadFinished = vi.fn();
                const { provider: p } = setupProvider({ onUploadFinished });
                const errorHandler = vi.fn();
                p.on('error', errorHandler);

                const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
                const { completion } = p.uploadFiles(files);
                const rejected = expect(completion).rejects.toThrow(/did not request the files/i);

                // No FileContentsRequest ever arrives; after the 60s lock window the
                // watchdog fires: resume monitoring + fail the upload.
                await vi.advanceTimersByTimeAsync(60_000);
                await rejected;

                expect(onUploadFinished).toHaveBeenCalledTimes(1);
                const err: FileTransferError = errorHandler.mock.calls.at(-1)![0];
                expect(err.direction).toBe('upload');

                // uploadState was cleared, so a fresh upload starts instead of throwing
                // "Upload already in progress" -- the wedge is gone, no reload needed.
                const second = p.uploadFiles(files);
                p.dispose();
                await expect(second.completion).rejects.toThrow('RdpFileTransferProvider disposed');
            } finally {
                vi.useRealTimers();
            }
        });

        it('does NOT fail a pulled upload that keeps making progress', async () => {
            // Regression guard for normal uploads: a slow-but-progressing transfer resets
            // the inactivity watchdog on every request, so it must never be killed even
            // long past the window.
            vi.useFakeTimers();
            try {
                const { provider: p } = setupProvider();
                const errorHandler = vi.fn();
                p.on('error', errorHandler);

                const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
                const { completion } = p.uploadFiles(files);
                let settled = false;
                void completion.then(
                    () => {
                        settled = true;
                    },
                    () => {
                        settled = true;
                    },
                );

                // Remote keeps pulling, each request well inside the 60s window: the
                // watchdog keeps resetting and never fires (total elapsed well past 60s).
                for (let i = 0; i < 5; i++) {
                    // @ts-expect-error - private callback the WASM layer drives
                    p.handleFileContentsRequest({ streamId: 1, index: 0, flags: 1, position: 0, size: 8 });
                    await vi.advanceTimersByTimeAsync(50_000);
                }

                expect(errorHandler).not.toHaveBeenCalled();
                expect(settled).toBe(false);

                p.dispose();
                await expect(completion).rejects.toThrow('RdpFileTransferProvider disposed');
            } finally {
                vi.useRealTimers();
            }
        });

        it('fails a pulled-then-idle upload after the inactivity window, releasing the wedge', async () => {
            // Once pulling starts, if the remote goes silent (e.g. it grabbed the clipboard
            // with a text/image copy that never reaches handleFilesAvailable), the
            // inactivity watchdog releases uploadState so later uploads aren't wedged.
            vi.useFakeTimers();
            try {
                const { provider: p } = setupProvider();
                const errorHandler = vi.fn();
                p.on('error', errorHandler);

                const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
                const { completion } = p.uploadFiles(files);
                const rejected = expect(completion).rejects.toThrow(/stopped requesting the files/i);

                // Remote pulls once (arms the inactivity watchdog), then goes silent.
                // @ts-expect-error - private callback the WASM layer drives
                p.handleFileContentsRequest({ streamId: 1, index: 0, flags: 1, position: 0, size: 8 });
                await vi.advanceTimersByTimeAsync(60_000);
                await rejected;

                expect(errorHandler).toHaveBeenCalled();
                expect((errorHandler.mock.calls.at(-1)![0] as FileTransferError).direction).toBe('upload');

                // uploadState released -> a fresh upload doesn't throw "Upload already in progress".
                const second = p.uploadFiles(files);
                p.dispose();
                await expect(second.completion).rejects.toThrow('RdpFileTransferProvider disposed');
            } finally {
                vi.useRealTimers();
            }
        });

        it('stops the inactivity watchdog once the upload completes (no lingering timer)', async () => {
            // The final FileContentsRequest arms the inactivity watchdog; completion must
            // disarm it (finishUploadBatch). Otherwise a stray 60s timer lingers after every
            // completed upload -- harmless today thanks to the uploadState guard, but it
            // should not exist, and a future change could let it fire against fresh state.
            vi.useFakeTimers();
            try {
                const { provider: p } = setupProvider();
                const errorHandler = vi.fn();
                p.on('error', errorHandler);

                const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
                const { completion } = p.uploadFiles(files);
                let resolved = false;
                void completion.then(() => {
                    resolved = true;
                });

                // Remote pulls the whole 4-byte file in one RANGE. jsdom schedules the
                // FileReader on a timer, so flushing it completes the batch.
                // @ts-expect-error - private callback the WASM layer drives
                p.handleFileContentsRequest({ streamId: 1, index: 0, flags: 2, position: 0, size: 4 });
                await vi.advanceTimersByTimeAsync(100);
                expect(resolved).toBe(true);

                // Completion ran finishUploadBatch, which cleared the inactivity watchdog:
                // no upload timer is left pending (the lingering-timer bug this guards).
                expect(vi.getTimerCount()).toBe(0);

                // ...and well past the window nothing fails.
                await vi.advanceTimersByTimeAsync(60_000);
                expect(errorHandler).not.toHaveBeenCalled();

                p.dispose();
            } finally {
                vi.useRealTimers();
            }
        });

        it('does NOT fail a re-paste that goes idle (isRePaste guard)', async () => {
            // After an upload completes, the DroppedFile metadata is retained so the remote
            // can re-paste. A re-paste rebuilds uploadState with isRePaste=true and carries
            // no external promise, so the inactivity watchdog must leave it alone rather than
            // emit a bogus upload error / try to reject a promise nobody is awaiting.
            vi.useFakeTimers();
            try {
                const { provider: p } = setupProvider();
                const errorHandler = vi.fn();
                p.on('error', errorHandler);

                const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
                const { completion } = p.uploadFiles(files);
                let resolved = false;
                void completion.then(() => {
                    resolved = true;
                });

                // First paste: remote pulls the whole file -> upload completes, metadata retained.
                // @ts-expect-error - private callback the WASM layer drives
                p.handleFileContentsRequest({ streamId: 1, index: 0, flags: 2, position: 0, size: 4 });
                await vi.advanceTimersByTimeAsync(100);
                expect(resolved).toBe(true);

                // Re-paste: no uploadState but retainedFiles present -> the next request
                // rebuilds an isRePaste state and re-arms the inactivity watchdog.
                // @ts-expect-error - private callback the WASM layer drives
                p.handleFileContentsRequest({ streamId: 2, index: 0, flags: 1, position: 0, size: 8 });

                // The window elapses with no further pulls. A fresh upload would be failed
                // here; a re-paste must NOT be (no promise to reject, no error to surface).
                await vi.advanceTimersByTimeAsync(60_000);
                expect(errorHandler).not.toHaveBeenCalled();

                p.dispose();
            } finally {
                vi.useRealTimers();
            }
        });

        it('releases the in-flight upload when the remote replaces the clipboard', async () => {
            // A remote FormatList (the remote copied its own files) supersedes our advertise;
            // the upload can no longer complete, so uploadState must be released or it wedges
            // every later upload. Symmetric to the rejected-advertise recovery.
            const { provider: p } = setupProvider();
            const errorHandler = vi.fn();
            p.on('error', errorHandler);

            const files = [new File(['data'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);
            const rejected = expect(completion).rejects.toThrow(/remote clipboard changed/i);

            // Remote copies its own files mid-upload.
            // @ts-expect-error - accessing private method for testing
            p.handleFilesAvailable([{ name: 'remote.txt', size: 10, lastModified: 0 }] as FileInfo[]);
            await rejected;

            expect(errorHandler).toHaveBeenCalled();
            expect((errorHandler.mock.calls.at(-1)![0] as FileTransferError).direction).toBe('upload');

            // uploadState released -> a fresh upload doesn't throw "Upload already in progress".
            const second = p.uploadFiles(files);
            p.dispose();
            await expect(second.completion).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('does NOT fail an in-flight upload on an Unlock (Unlock is not a paste-failure signal)', async () => {
            // The peer/cliprdr emit Unlock routinely (snapshot of the FormatList, and the
            // 60s timeout), often before any FileContentsRequest. Treating that as failure
            // tore down uploads that would still be pulled, so Unlock must be ignored here.
            const { provider: p } = setupProvider();
            const errorHandler = vi.fn();
            p.on('error', errorHandler);

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);

            // @ts-expect-error - private callback the WASM layer drives via unlockCallback
            p.handleUnlock(0);

            // No error, and the upload state is intact (a second attempt still hits the guard).
            expect(errorHandler).not.toHaveBeenCalled();
            expect(() => p.uploadFiles(files)).toThrow('Upload already in progress');

            p.dispose();
            await expect(completion).rejects.toThrow('RdpFileTransferProvider disposed');
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
            // The failure path resumes monitoring (resumeUploadMonitoring), so it fires once.
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });
    });

    describe('format list response (paste accept / reject)', () => {
        // Pull the registered format_list_response_callback out of the builder
        // extensions and invoke it, exercising the real wiring + handler together.
        function fireFormatListResponse(p: RdpFileTransferProviderInstance, ok: boolean): void {
            const exts = p.getBuilderExtensions() as unknown as Array<{ ident: string; value: (ok: boolean) => void }>;
            const ext = exts.find((e) => e.ident === 'format_list_response_callback');
            if (ext === undefined) {
                throw new Error('format_list_response_callback was not registered');
            }
            ext.value(ok);
        }

        it('registers a format_list_response_callback builder extension', () => {
            const idents = (provider.getBuilderExtensions() as unknown as Array<{ ident: string }>).map((e) => e.ident);
            expect(idents).toContain('format_list_response_callback');
        });

        it('emits a format-list-response event carrying the ok flag', () => {
            const handler = vi.fn();
            provider.on('format-list-response', handler);

            fireFormatListResponse(provider, true);
            fireFormatListResponse(provider, false);

            expect(handler).toHaveBeenNthCalledWith(1, true);
            expect(handler).toHaveBeenNthCalledWith(2, false);
        });

        it('fails an in-flight upload cleanly when the remote rejects the advertise', async () => {
            const { provider: p, session: s } = setupProvider();
            const errorHandler = vi.fn();
            p.on('error', errorHandler);

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);

            // Remote rejects the file list before requesting any contents.
            fireFormatListResponse(p, false);

            await expect(completion).rejects.toThrow(/rejected the file list/i);
            const err: FileTransferError = errorHandler.mock.calls.at(-1)![0];
            expect(err.direction).toBe('upload');

            // uploadState is cleared, so a fresh upload starts instead of throwing
            // "Upload already in progress" (the wedge this fixes). Reaching a handle
            // (not a throw) plus the initiateFileCopy call proves it restarted.
            s.invokeExtension.mockClear();
            const second = p.uploadFiles(files);
            expect(s.invokeExtension).toHaveBeenCalledTimes(1);

            p.dispose();
            await expect(second.completion).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('leaves an accepted advertise (ok=true) untouched', async () => {
            const { provider: p } = setupProvider();
            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion } = p.uploadFiles(files);

            fireFormatListResponse(p, true);

            // Upload is still in progress: a second attempt still hits the guard.
            expect(() => p.uploadFiles(files)).toThrow('Upload already in progress');

            p.dispose();
            await expect(completion).rejects.toThrow('RdpFileTransferProvider disposed');
        });

        it('surfaces a reject with no upload in progress without erroring', () => {
            const errorHandler = vi.fn();
            const eventHandler = vi.fn();
            provider.on('error', errorHandler);
            provider.on('format-list-response', eventHandler);

            expect(() => fireFormatListResponse(provider, false)).not.toThrow();
            expect(eventHandler).toHaveBeenCalledWith(false);
            expect(errorHandler).not.toHaveBeenCalled();
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
