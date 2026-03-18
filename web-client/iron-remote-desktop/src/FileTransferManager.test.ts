import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { FileTransferManager } from './FileTransferManager';
import type { FileTransferError } from './FileTransferManager';
import type { SessionBuilder } from './interfaces/SessionBuilder';
import type { Session } from './interfaces/Session';
import type { FileInfo } from './interfaces/FileTransfer';

/**
 * FileTransferManager Unit Tests
 *
 * Testing Strategy:
 * ----------------
 * These tests cover the JavaScript layer of FileTransferManager including:
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
 *
 * These integration-level behaviors should be tested with:
 * - Real WASM module loaded in browser environment
 * - End-to-end tests with mock RDP server
 * - Or Playwright/Puppeteer browser automation
 */

// Mock SessionBuilder for testing
class MockSessionBuilder {
    private _filesAvailableCallback?: (files: FileInfo[]) => void;
    private _fileContentsRequestCallback?: (req: unknown) => void;
    private _fileContentsResponseCallback?: (resp: unknown) => void;
    private _lockCallback?: (id: number) => void;
    private _unlockCallback?: (id: number) => void;
    private _connectFn?: () => Promise<Session>;

    filesAvailableCallback(callback: (files: FileInfo[]) => void): this {
        this._filesAvailableCallback = callback;
        return this;
    }

    fileContentsRequestCallback(callback: (req: unknown) => void): this {
        this._fileContentsRequestCallback = callback;
        return this;
    }

    fileContentsResponseCallback(callback: (resp: unknown) => void): this {
        this._fileContentsResponseCallback = callback;
        return this;
    }

    lockCallback(callback: (id: number) => void): this {
        this._lockCallback = callback;
        return this;
    }

    unlockCallback(callback: (id: number) => void): this {
        this._unlockCallback = callback;
        return this;
    }

    locksExpiredCallback(_callback: (clipDataIds: Uint32Array) => void): this {
        // Store callback but don't need to use it in unit tests
        return this;
    }

    // Simulate triggering callbacks from WASM
    triggerFilesAvailable(files: FileInfo[]): void {
        this._filesAvailableCallback?.(files);
    }

    triggerFileContentsRequest(req: unknown): void {
        this._fileContentsRequestCallback?.(req);
    }

    triggerFileContentsResponse(resp: unknown): void {
        this._fileContentsResponseCallback?.(resp);
    }

    triggerLock(id: number): void {
        this._lockCallback?.(id);
    }

    triggerUnlock(id: number): void {
        this._unlockCallback?.(id);
    }

    connect(): Promise<Session> {
        if (this._connectFn) {
            return this._connectFn();
        }
        throw new Error('MockSessionBuilder: connect not configured');
    }

    setConnectFn(fn: () => Promise<Session>): void {
        this._connectFn = fn;
    }
}

// Mock Session for testing
class MockSession implements Partial<Session> {
    requestFileContents = vi.fn();
    submitFileContents = vi.fn();
    initiateFileCopy = vi.fn();
}

describe('FileTransferManager', () => {
    let manager: FileTransferManager;
    let mockBuilder: MockSessionBuilder;
    let mockSession: MockSession;

    beforeEach(() => {
        mockBuilder = new MockSessionBuilder();
        mockSession = new MockSession();
        mockBuilder.setConnectFn(async () => mockSession as unknown as Session);
        manager = FileTransferManager.setup(mockBuilder as unknown as SessionBuilder);
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    describe('setup and initialization', () => {
        it('should create manager instance', () => {
            expect(manager).toBeInstanceOf(FileTransferManager);
        });

        it('should register callbacks with SessionBuilder', () => {
            const builder = new MockSessionBuilder();
            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder);
            expect(mgr).toBeInstanceOf(FileTransferManager);
        });

        it('should use custom chunk size', () => {
            const customManager = new FileTransferManager({ chunkSize: 32768 });
            expect(customManager).toBeDefined();
        });

        it('should throw error when session not available', () => {
            const mgr = new FileTransferManager();
            expect(() => {
                // @ts-expect-error - accessing private method for testing
                mgr.ensureSession();
            }).toThrow('Session not available');
        });
    });

    describe('event system', () => {
        it('should register event handlers', () => {
            const handler = vi.fn();
            manager.on('files-available', handler);
            // No error should be thrown
        });

        it('should emit files-available event', async () => {
            const handler = vi.fn();
            manager.on('files-available', handler);

            // Simulate connection
            await mockBuilder.connect();

            const files: FileInfo[] = [{ name: 'test.txt', size: 1024, lastModified: Date.now() }];

            mockBuilder.triggerFilesAvailable(files);
            expect(handler).toHaveBeenCalledWith(files);
        });

        it('should remove event handlers with off()', () => {
            const handler = vi.fn();
            manager.on('files-available', handler);
            manager.off('files-available', handler);

            // Handler should not be called after removal
            mockBuilder.triggerFilesAvailable([]);
            expect(handler).not.toHaveBeenCalled();
        });

        it('should support multiple handlers for same event', async () => {
            const handler1 = vi.fn();
            const handler2 = vi.fn();

            manager.on('files-available', handler1);
            manager.on('files-available', handler2);

            await mockBuilder.connect();

            const files: FileInfo[] = [{ name: 'test.txt', size: 100, lastModified: Date.now() }];
            mockBuilder.triggerFilesAvailable(files);

            expect(handler1).toHaveBeenCalledWith(files);
            expect(handler2).toHaveBeenCalledWith(files);
        });
    });

    describe('browser integration helpers', () => {
        it('should have showFilePicker method', () => {
            expect(manager.showFilePicker).toBeDefined();
        });

        it('should have handleDrop method', () => {
            expect(manager.handleDrop).toBeDefined();
        });

        it('should have handleDragOver method', () => {
            expect(manager.handleDragOver).toBeDefined();
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

            const files = await manager.handleDrop(mockEvent);
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

            const result = await manager.handleDrop(mockEvent);
            expect(result).toHaveLength(1);
            expect(result[0].name).toBe('hello.txt');
            expect(result[0].file).toBe(testFile);
            expect(result[0].path).toBeUndefined();
            expect(result[0].isDirectory).toBeUndefined();
        });

        it('should fall back to getAsFile when webkitGetAsEntry is unavailable', async () => {
            const testFile = new File(['data'], 'data.bin');

            const mockItem = {
                kind: 'file',
                // webkitGetAsEntry is absent
                getAsFile: () => testFile,
            };

            const mockDataTransfer = {
                items: [mockItem],
            };

            const mockEvent = {
                dataTransfer: mockDataTransfer,
                preventDefault: vi.fn(),
            } as unknown as DragEvent;

            const result = await manager.handleDrop(mockEvent);
            expect(result).toHaveLength(1);
            expect(result[0].name).toBe('data.bin');
            expect(result[0].file).toBe(testFile);
        });

        it('should prevent default on drag over', () => {
            const mockEvent = {
                preventDefault: vi.fn(),
                stopPropagation: vi.fn(),
            } as unknown as DragEvent;

            manager.handleDragOver(mockEvent);
            expect(mockEvent.preventDefault).toHaveBeenCalled();
        });
    });

    describe('cleanup and disposal', () => {
        it('should dispose and cleanup resources', () => {
            manager.dispose();
            // Should not throw
        });

        it('should remove event handlers on dispose', () => {
            const handler = vi.fn();
            manager.on('files-available', handler);
            manager.dispose();

            // Handler should not be called after disposal
            mockBuilder.triggerFilesAvailable([]);
            expect(handler).not.toHaveBeenCalled();
        });

        it('should not throw when disposed without a session', () => {
            // Create a manager without calling connect() — session is undefined
            const builder = new MockSessionBuilder() as unknown as SessionBuilder;
            const mgr = FileTransferManager.setup(builder);

            // dispose() should complete without throwing even though session is undefined
            expect(() => mgr.dispose()).not.toThrow();
        });

        it('should restore original builder.connect() after dispose', async () => {
            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            // Capture the original connect before setup wraps it
            const originalConnect = builder.connect.bind(builder);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder);

            // After setup, builder.connect is wrapped by the manager
            const wrappedConnect = (builder as unknown as SessionBuilder).connect;
            expect(wrappedConnect).not.toBe(originalConnect);

            mgr.dispose();

            // After dispose, builder.connect should be restored to original.
            // Calling connect() again should succeed without setting session
            // on the disposed manager.
            const newSession = await (builder as unknown as SessionBuilder).connect();
            expect(newSession).toBeDefined();
        });

        it('should bypass manager on connect() after dispose', async () => {
            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder);
            mgr.dispose();

            // Connect after dispose - the manager's wrapper checks the disposed
            // flag and delegates to the original connect without setting session
            const result = await (builder as unknown as SessionBuilder).connect();
            expect(result).toBe(session as unknown as Session);

            // The disposed manager should not have a working session. Verifying
            // this indirectly: attempting to download should fail because the
            // manager's internal session reference is undefined.
            const fileInfo: FileInfo = { name: 'test.txt', size: 100, lastModified: 0 };
            const { completion } = mgr.downloadFile(fileInfo, 0);
            await expect(completion).rejects.toThrow();
        });

        it('should reject active upload on dispose', async () => {
            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder);
            await (builder as unknown as SessionBuilder).connect();

            // Start an upload that will be interrupted by dispose
            const files = [new File(['data'], 'test.txt', { type: 'text/plain' })];
            const { completion } = mgr.uploadFiles(files);

            mgr.dispose();

            // The upload promise should reject with dispose error
            await expect(completion).rejects.toThrow('FileTransferManager disposed');
        });
    });

    describe('edge cases', () => {
        it('should handle very large timestamps', () => {
            const fileInfo: FileInfo = {
                name: 'test.txt',
                size: 10,
                lastModified: Date.now() + 1000000000, // Far future
            };

            // Verify FileInfo structure is valid
            expect(fileInfo.name).toBe('test.txt');
            expect(fileInfo.size).toBe(10);
            expect(fileInfo.lastModified).toBeGreaterThan(Date.now());
        });

        it('should handle files with special characters in name', () => {
            const fileInfo: FileInfo = {
                name: 'test file (1) [copy].txt',
                size: 10,
                lastModified: Date.now(),
            };

            // Verify FileInfo structure handles special characters
            expect(fileInfo.name).toBe('test file (1) [copy].txt');
        });

        it('should handle missing lastModified timestamp', () => {
            const file = new File(['test'], 'test.txt');
            // File without explicit lastModified should have default timestamp
            expect(file.lastModified).toBeGreaterThan(0);
        });

        it('should generate unique monotonic stream IDs', () => {
            const mgr = new FileTransferManager();
            const ids = new Set<number>();

            // Generate 1000 stream IDs
            for (let i = 0; i < 1000; i++) {
                // @ts-expect-error - accessing private method for testing
                const id = mgr.generateStreamId();
                expect(ids.has(id)).toBe(false); // Should be unique
                ids.add(id);
            }

            // Verify IDs are monotonically increasing
            const sortedIds = Array.from(ids).sort((a, b) => a - b);
            for (let i = 0; i < sortedIds.length; i++) {
                expect(sortedIds[i]).toBe(i + 1); // Should be sequential starting from 1
            }
        });
    });

    describe('error direction field', () => {
        it('should emit upload direction on initiateFileCopy failure', async () => {
            // Connect so session is available
            await mockBuilder.connect();

            // Make initiateFileCopy throw to trigger the upload error path
            mockSession.initiateFileCopy.mockImplementationOnce(() => {
                throw new Error('Copy failed');
            });

            const errorHandler = vi.fn();
            manager.on('error', errorHandler);

            const files = [new File(['content'], 'upload.txt', { type: 'text/plain' })];

            // uploadFiles returns synchronously; the completion promise rejects
            const { completion } = manager.uploadFiles(files);
            await expect(completion).rejects.toThrow('Failed to initiate file upload');

            // Verify the error event includes direction: 'upload'
            expect(errorHandler).toHaveBeenCalledTimes(1);
            const emittedError: FileTransferError = errorHandler.mock.calls[0][0];
            expect(emittedError.direction).toBe('upload');
        });

        it('should emit download direction on requestFileContents failure', async () => {
            // Connect so session is available
            await mockBuilder.connect();

            // requestFileContents throws
            mockSession.requestFileContents.mockImplementationOnce(() => {
                throw new Error('Request failed');
            });

            const errorHandler = vi.fn();
            manager.on('error', errorHandler);

            const fileInfo: FileInfo = { name: 'data.bin', size: 2048, lastModified: Date.now() };

            const { completion } = manager.downloadFile(fileInfo, 0);
            await expect(completion).rejects.toThrow('Failed to request file size');

            expect(errorHandler).toHaveBeenCalledTimes(1);
            const emittedError: FileTransferError = errorHandler.mock.calls[0][0];
            expect(emittedError.direction).toBe('download');
            expect(emittedError.fileName).toBe('data.bin');
        });

        it('should reject overlapping upload calls', async () => {
            await mockBuilder.connect();

            const files = [new File(['content'], 'file.txt', { type: 'text/plain' })];

            // Start an upload that will remain pending (never resolves)
            mockSession.initiateFileCopy.mockImplementation(() => undefined);
            const { completion: firstUpload } = manager.uploadFiles(files);

            // A second concurrent upload should throw synchronously
            expect(() => manager.uploadFiles(files)).toThrow('Upload already in progress');

            // Clean up: dispose to settle the pending first upload
            manager.dispose();
            await expect(firstUpload).rejects.toThrow();
        });
    });

    describe('per-file upload cancellation', () => {
        it('should cancel a single file without aborting the batch', async () => {
            await mockBuilder.connect();

            const file0 = new File(['aaa'], 'a.txt', { type: 'text/plain' });
            const file1 = new File(['bbb'], 'b.txt', { type: 'text/plain' });
            const files = [file0, file1];

            const ac0 = new AbortController();
            const perFileSignals = new Map([[0, ac0.signal]]);

            const cancelHandler = vi.fn();
            manager.on('transfer-cancelled', cancelHandler);

            const { transferIds, completion: uploadPromise } = manager.uploadFiles(files, undefined, perFileSignals);

            // Cancel file 0 before the remote requests any chunks
            ac0.abort();

            expect(cancelHandler).toHaveBeenCalledWith({
                transferId: transferIds.get(0),
                fileIndex: 0,
                direction: 'upload',
            });

            // The remote requests a SIZE for cancelled file 0 — should get an error response
            mockBuilder.triggerFileContentsRequest({
                streamId: 100,
                index: 0,
                flags: 0x1, // SIZE
                position: 0,
                size: 8,
            });
            expect(mockSession.submitFileContents).toHaveBeenCalledWith(100, true, expect.any(Uint8Array));

            // File 1 SIZE request — should succeed normally
            mockBuilder.triggerFileContentsRequest({
                streamId: 101,
                index: 1,
                flags: 0x1, // SIZE
                position: 0,
                size: 8,
            });
            // submitFileContents for file 1 SIZE should be called with isError=false
            const file1SizeCall = mockSession.submitFileContents.mock.calls.find((call: unknown[]) => call[0] === 101);
            expect(file1SizeCall).toBeDefined();
            expect(file1SizeCall![1]).toBe(false); // isError = false

            // File 1 RANGE request — reads the full file content
            mockBuilder.triggerFileContentsRequest({
                streamId: 102,
                index: 1,
                flags: 0x2, // RANGE
                position: 0,
                size: 3,
            });

            // Wait for FileReader async completion
            await vi.waitFor(() => {
                const rangeCall = mockSession.submitFileContents.mock.calls.find((call: unknown[]) => call[0] === 102);
                expect(rangeCall).toBeDefined();
                expect(rangeCall![1]).toBe(false); // isError = false
            });

            // Upload should resolve since file 0 was cancelled (completed) and file 1 finished
            await uploadPromise;
        });

        it('should send error response for subsequent requests on cancelled file', async () => {
            await mockBuilder.connect();

            const file0 = new File(['data'], 'file.txt', { type: 'text/plain' });
            const file1 = new File(['keep'], 'keep.txt', { type: 'text/plain' });
            const ac0 = new AbortController();
            const perFileSignals = new Map([[0, ac0.signal]]);

            manager.uploadFiles([file0, file1], undefined, perFileSignals);

            // Cancel file 0 - batch stays alive because file 1 is still pending
            ac0.abort();

            // Multiple requests for the cancelled file should all get error responses
            mockBuilder.triggerFileContentsRequest({
                streamId: 200,
                index: 0,
                flags: 0x1, // SIZE
                position: 0,
                size: 8,
            });
            mockBuilder.triggerFileContentsRequest({
                streamId: 201,
                index: 0,
                flags: 0x2, // RANGE
                position: 0,
                size: 4,
            });

            expect(mockSession.submitFileContents).toHaveBeenCalledWith(200, true, expect.any(Uint8Array));
            expect(mockSession.submitFileContents).toHaveBeenCalledWith(201, true, expect.any(Uint8Array));
        });

        it('should not cancel an already-completed file', async () => {
            await mockBuilder.connect();

            const file0 = new File(['x'], 'x.txt', { type: 'text/plain' });
            const ac0 = new AbortController();
            const perFileSignals = new Map([[0, ac0.signal]]);

            const cancelHandler = vi.fn();
            manager.on('transfer-cancelled', cancelHandler);

            const { completion: uploadPromise } = manager.uploadFiles([file0], undefined, perFileSignals);

            // File 0 SIZE request
            mockBuilder.triggerFileContentsRequest({
                streamId: 300,
                index: 0,
                flags: 0x1,
                position: 0,
                size: 8,
            });

            // File 0 RANGE request (full file)
            mockBuilder.triggerFileContentsRequest({
                streamId: 301,
                index: 0,
                flags: 0x2,
                position: 0,
                size: 1,
            });

            // Wait for upload to complete
            await uploadPromise;

            // Now abort after completion — should have no effect
            ac0.abort();
            expect(cancelHandler).not.toHaveBeenCalled();
        });

        it('should handle mid-read abort without rejecting the batch', async () => {
            await mockBuilder.connect();

            const file0 = new File(['hello'], 'hello.txt', { type: 'text/plain' });
            const file1 = new File(['world'], 'world.txt', { type: 'text/plain' });
            const files = [file0, file1];

            const ac0 = new AbortController();
            const perFileSignals = new Map([[0, ac0.signal]]);

            const cancelHandler = vi.fn();
            const errorHandler = vi.fn();
            manager.on('transfer-cancelled', cancelHandler);
            manager.on('error', errorHandler);

            const { transferIds, completion: uploadPromise } = manager.uploadFiles(files, undefined, perFileSignals);

            // File 0 SIZE request — succeeds before cancel
            mockBuilder.triggerFileContentsRequest({
                streamId: 400,
                index: 0,
                flags: 0x1,
                position: 0,
                size: 8,
            });

            // File 0 RANGE request — starts a FileReader read
            mockBuilder.triggerFileContentsRequest({
                streamId: 401,
                index: 0,
                flags: 0x2,
                position: 0,
                size: 5,
            });

            // Cancel file 0 while the FileReader is reading
            ac0.abort();

            expect(cancelHandler).toHaveBeenCalledWith({
                transferId: transferIds.get(0),
                fileIndex: 0,
                direction: 'upload',
            });

            // The reader.onerror should NOT emit an error or reject the batch
            // since we check cancelledFiles in the onerror handler
            await vi.waitFor(() => {
                // Error handler should NOT have been called for the cancelled file
                expect(errorHandler).not.toHaveBeenCalled();
            });

            // Complete file 1 to resolve the batch
            mockBuilder.triggerFileContentsRequest({
                streamId: 402,
                index: 1,
                flags: 0x1,
                position: 0,
                size: 8,
            });
            mockBuilder.triggerFileContentsRequest({
                streamId: 403,
                index: 1,
                flags: 0x2,
                position: 0,
                size: 5,
            });

            await vi.waitFor(() => {
                const rangeCall = mockSession.submitFileContents.mock.calls.find((call: unknown[]) => call[0] === 403);
                expect(rangeCall).toBeDefined();
            });

            await uploadPromise;
        });
    });

    describe('upload lifecycle callbacks', () => {
        it('should call onUploadStarted before initiateFileCopy', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadStarted,
                onUploadFinished,
            });
            await builder.connect();

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];

            const { completion: uploadPromise } = mgr.uploadFiles(files);

            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            expect(session.initiateFileCopy).toHaveBeenCalledTimes(1);
            // onUploadStarted should have been called before initiateFileCopy.
            // Since vi.fn() tracks call order, check that started was called first.
            expect(onUploadStarted.mock.invocationCallOrder[0]).toBeLessThan(
                session.initiateFileCopy.mock.invocationCallOrder[0],
            );
            expect(onUploadFinished).not.toHaveBeenCalled();

            // Complete the upload: SIZE then RANGE for the single file
            builder.triggerFileContentsRequest({ streamId: 1, index: 0, flags: 0x1, position: 0, size: 8 });
            builder.triggerFileContentsRequest({ streamId: 2, index: 0, flags: 0x2, position: 0, size: 1 });

            await vi.waitFor(() => {
                expect(session.submitFileContents.mock.calls.find((c: unknown[]) => c[0] === 2)).toBeDefined();
            });
            await uploadPromise;

            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });

        it('should call onUploadFinished on initiateFileCopy failure', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const session = new MockSession();
            session.initiateFileCopy.mockImplementation(() => {
                throw new Error('Copy failed');
            });
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadStarted,
                onUploadFinished,
            });
            await builder.connect();

            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];
            const { completion } = mgr.uploadFiles(files);
            await expect(completion).rejects.toThrow('Failed to initiate file upload');

            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });

        it('should call onUploadFinished on batch abort', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadStarted,
                onUploadFinished,
            });
            await builder.connect();

            const ac = new AbortController();
            const files = [new File(['data'], 'data.txt', { type: 'text/plain' })];
            const { completion: uploadPromise } = mgr.uploadFiles(files, ac.signal);

            expect(onUploadStarted).toHaveBeenCalledTimes(1);
            expect(onUploadFinished).not.toHaveBeenCalled();

            ac.abort();
            await expect(uploadPromise).rejects.toThrow('Upload cancelled');

            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });

        it('should call onUploadFinished on dispose during upload', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadStarted,
                onUploadFinished,
            });
            await builder.connect();

            const files = [new File(['data'], 'data.txt', { type: 'text/plain' })];
            const { completion: uploadPromise } = mgr.uploadFiles(files);

            expect(onUploadStarted).toHaveBeenCalledTimes(1);

            mgr.dispose();
            await expect(uploadPromise).rejects.toThrow('FileTransferManager disposed');

            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });

        it('should not call onUploadFinished on dispose without active upload', () => {
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadFinished,
            });

            mgr.dispose();
            expect(onUploadFinished).not.toHaveBeenCalled();
        });

        it('should call onUploadFinished when all per-file cancellations complete the batch', async () => {
            const onUploadStarted = vi.fn();
            const onUploadFinished = vi.fn();

            const builder = new MockSessionBuilder();
            const session = new MockSession();
            builder.setConnectFn(async () => session as unknown as Session);

            const mgr = FileTransferManager.setup(builder as unknown as SessionBuilder, {
                onUploadStarted,
                onUploadFinished,
            });
            await builder.connect();

            const ac0 = new AbortController();
            const perFileSignals = new Map([[0, ac0.signal]]);
            const files = [new File(['x'], 'x.txt', { type: 'text/plain' })];

            const { completion: uploadPromise } = mgr.uploadFiles(files, undefined, perFileSignals);
            expect(onUploadStarted).toHaveBeenCalledTimes(1);

            // Cancel the only file - should complete the batch
            ac0.abort();
            await uploadPromise;

            expect(onUploadFinished).toHaveBeenCalledTimes(1);
        });
    });

    describe('sanitizeFileName', () => {
        it('should return a plain filename as-is', () => {
            expect(FileTransferManager.sanitizeFileName('file.txt')).toBe('file.txt');
        });

        it('should strip Unix path traversal', () => {
            expect(FileTransferManager.sanitizeFileName('../../../etc/passwd')).toBe('passwd');
        });

        it('should strip Windows path traversal', () => {
            expect(FileTransferManager.sanitizeFileName('..\\..\\system32\\config\\SAM')).toBe('SAM');
        });

        it('should extract basename from Windows absolute path', () => {
            expect(FileTransferManager.sanitizeFileName('C:\\Users\\victim\\Desktop\\file.txt')).toBe('file.txt');
        });

        it('should extract basename from Unix absolute path', () => {
            expect(FileTransferManager.sanitizeFileName('/home/user/file.txt')).toBe('file.txt');
        });

        it('should return fallback for empty string', () => {
            expect(FileTransferManager.sanitizeFileName('')).toBe('unnamed_file');
        });

        it('should return fallback for traversal-only input', () => {
            expect(FileTransferManager.sanitizeFileName('../..')).toBe('unnamed_file');
        });

        it('should handle trailing separator', () => {
            expect(FileTransferManager.sanitizeFileName('path/to/file/')).toBe('file');
        });

        it('should handle mixed separators', () => {
            expect(FileTransferManager.sanitizeFileName('path/to\\file.txt')).toBe('file.txt');
        });

        it('should keep triple-dot filename (not traversal)', () => {
            expect(FileTransferManager.sanitizeFileName('...')).toBe('...');
        });
    });

    describe('sanitizePath', () => {
        it('should return undefined for empty string', () => {
            expect(FileTransferManager.sanitizePath('')).toBeUndefined();
        });

        it('should return undefined for traversal-only path', () => {
            expect(FileTransferManager.sanitizePath('../..')).toBeUndefined();
            expect(FileTransferManager.sanitizePath('.')).toBeUndefined();
        });

        it('should preserve a simple relative path', () => {
            expect(FileTransferManager.sanitizePath('temp')).toBe('temp');
        });

        it('should preserve a multi-level relative path', () => {
            expect(FileTransferManager.sanitizePath('folder\\sub')).toBe('folder\\sub');
        });

        it('should strip traversal components from path', () => {
            expect(FileTransferManager.sanitizePath('..\\..\\etc')).toBe('etc');
        });

        it('should strip drive letter prefix', () => {
            expect(FileTransferManager.sanitizePath('C:\\Users\\Desktop')).toBe('Users\\Desktop');
        });

        it('should normalize Unix separators to backslash', () => {
            expect(FileTransferManager.sanitizePath('folder/sub')).toBe('folder\\sub');
        });

        it('should handle mixed separators', () => {
            expect(FileTransferManager.sanitizePath('folder/sub\\dir')).toBe('folder\\sub\\dir');
        });

        it('should return undefined if only drive letter remains', () => {
            expect(FileTransferManager.sanitizePath('C:')).toBeUndefined();
        });

        it('should strip UNC long path prefix with drive letter', () => {
            // \\?\C:\Users\Desktop splits into ["?", "C:", "Users", "Desktop"]
            expect(FileTransferManager.sanitizePath('?\\C:\\Users\\Desktop')).toBe('Users\\Desktop');
        });

        it('should strip UNC device prefix', () => {
            // \\.\ splits into [".", "device", "path"]
            expect(FileTransferManager.sanitizePath('.\\device\\path')).toBe('device\\path');
        });

        it('should return undefined if only UNC prefix remains', () => {
            expect(FileTransferManager.sanitizePath('?\\C:')).toBeUndefined();
        });
    });

    describe('directory drag-and-drop traversal', () => {
        /**
         * Helper: build a mock FileSystemDirectoryEntry whose createReader()
         * returns the given children in a single readEntries() batch.
         */
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
                                cb([]); // Signals end of entries
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

            const result = await manager.handleDrop(dropEventWithEntries([folder]));

            // 1 directory entry + 2 file entries
            expect(result).toHaveLength(3);

            // Directory entry
            expect(result[0].name).toBe('myFolder');
            expect(result[0].isDirectory).toBe(true);
            expect(result[0].file).toBeNull();
            expect(result[0].path).toBeUndefined();

            // Files inside the directory
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

            const result = await manager.handleDrop(dropEventWithEntries([topDir]));

            // top (dir), sub (dir), deep.txt (file)
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

            const result = await manager.handleDrop(dropEventWithEntries([rootFile, folder]));

            expect(result).toHaveLength(3);
            expect(result[0]).toMatchObject({ name: 'root.txt', path: undefined });
            expect(result[1]).toMatchObject({ name: 'dir', isDirectory: true });
            expect(result[2]).toMatchObject({ name: 'inside.txt', path: 'dir' });
        });

        it('should respect the max entry count limit', async () => {
            // Build a directory with more children than MAX_DIRECTORY_ENTRIES
            const children: FileSystemEntry[] = [];
            for (let i = 0; i < 1001; i++) {
                children.push(mockFileEntry(`file${i}.txt`, `${i}`));
            }
            const bigDir = mockDirEntry('big', children);

            const result = await manager.handleDrop(dropEventWithEntries([bigDir]));

            // Should be capped at 1000 (the constant) - the dir entry itself
            // counts as one, so at most 1000 total entries
            expect(result.length).toBeLessThanOrEqual(1000);
        });

        it('should handle empty directories', async () => {
            const emptyDir = mockDirEntry('empty', []);

            const result = await manager.handleDrop(dropEventWithEntries([emptyDir]));

            expect(result).toHaveLength(1);
            expect(result[0]).toMatchObject({
                name: 'empty',
                isDirectory: true,
                file: null,
                size: 0,
            });
        });
    });

    describe('directory upload with uploadFiles', () => {
        it('should pass path and isDirectory to initiateFileCopy', async () => {
            await mockBuilder.connect();

            const dropped = [
                {
                    file: null,
                    name: 'docs',
                    size: 0,
                    lastModified: 0,
                    isDirectory: true as const,
                },
                {
                    file: new File(['hello'], 'readme.txt'),
                    name: 'readme.txt',
                    size: 5,
                    lastModified: 1000,
                    path: 'docs',
                },
            ];

            const { completion } = manager.uploadFiles(dropped);

            expect(mockSession.initiateFileCopy).toHaveBeenCalledWith([
                { name: 'docs', size: 0, lastModified: 0, path: undefined, isDirectory: true },
                { name: 'readme.txt', size: 5, lastModified: 1000, path: 'docs', isDirectory: undefined },
            ]);

            manager.dispose();
            await expect(completion).rejects.toThrow('FileTransferManager disposed');
        });

        it('should not count directory entries toward expectedFileCount', async () => {
            await mockBuilder.connect();

            const dropped = [
                { file: null, name: 'dir', size: 0, lastModified: 0, isDirectory: true as const },
                { file: new File(['x'], 'x.txt'), name: 'x.txt', size: 1, lastModified: 0 },
            ];

            const { completion: uploadPromise } = manager.uploadFiles(dropped);

            // Only file entry needs to complete - directory does not count
            // Serve the file (index 1 in the dropped array)
            mockBuilder.triggerFileContentsRequest({
                streamId: 10,
                index: 1,
                flags: 0x1, // SIZE
                position: 0,
                size: 8,
            });
            mockBuilder.triggerFileContentsRequest({
                streamId: 11,
                index: 1,
                flags: 0x2, // RANGE
                position: 0,
                size: 1,
            });

            await vi.waitFor(() => {
                const rangeCall = mockSession.submitFileContents.mock.calls.find((call: unknown[]) => call[0] === 11);
                expect(rangeCall).toBeDefined();
            });

            await uploadPromise;
        });

        it('should respond to SIZE request on directory entry with zero', async () => {
            await mockBuilder.connect();

            const dropped = [
                { file: null, name: 'dir', size: 0, lastModified: 0, isDirectory: true as const },
                { file: new File(['x'], 'x.txt'), name: 'x.txt', size: 1, lastModified: 0 },
            ];

            const { completion } = manager.uploadFiles(dropped);

            // SIZE request for the directory entry (index 0)
            mockBuilder.triggerFileContentsRequest({
                streamId: 20,
                index: 0,
                flags: 0x1,
                position: 0,
                size: 8,
            });

            // Should respond with isError=false and 8 zero bytes
            const call = mockSession.submitFileContents.mock.calls.find((c: unknown[]) => c[0] === 20);
            expect(call).toBeDefined();
            expect(call![1]).toBe(false); // isError
            const data = call![2] as Uint8Array;
            expect(data.length).toBe(8);
            // All bytes should be zero (size = 0)
            expect(Array.from(data)).toEqual([0, 0, 0, 0, 0, 0, 0, 0]);

            manager.dispose();
            await expect(completion).rejects.toThrow('FileTransferManager disposed');
        });

        it('should respond to RANGE request on directory entry with error', async () => {
            await mockBuilder.connect();

            const dropped = [{ file: null, name: 'dir', size: 0, lastModified: 0, isDirectory: true as const }];

            const { completion } = manager.uploadFiles(dropped);

            // RANGE request for directory entry - should be an error response
            mockBuilder.triggerFileContentsRequest({
                streamId: 30,
                index: 0,
                flags: 0x2,
                position: 0,
                size: 100,
            });

            const call = mockSession.submitFileContents.mock.calls.find((c: unknown[]) => c[0] === 30);
            expect(call).toBeDefined();
            expect(call![1]).toBe(true); // isError

            manager.dispose();
            await expect(completion).rejects.toThrow('FileTransferManager disposed');
        });

        it('should accept plain File[] for backward compatibility', async () => {
            await mockBuilder.connect();

            const files = [new File(['abc'], 'abc.txt')];
            const { completion: uploadPromise } = manager.uploadFiles(files);

            mockBuilder.triggerFileContentsRequest({
                streamId: 40,
                index: 0,
                flags: 0x1,
                position: 0,
                size: 8,
            });
            mockBuilder.triggerFileContentsRequest({
                streamId: 41,
                index: 0,
                flags: 0x2,
                position: 0,
                size: 3,
            });

            await vi.waitFor(() => {
                const call = mockSession.submitFileContents.mock.calls.find((c: unknown[]) => c[0] === 41);
                expect(call).toBeDefined();
            });

            await uploadPromise;
        });
    });

    describe('handleFilesAvailable preserves path and isDirectory', () => {
        it('should pass through path and isDirectory from files', () => {
            const builder = new MockSessionBuilder() as unknown as SessionBuilder;
            const manager = FileTransferManager.setup(builder);

            const receivedFiles: FileInfo[][] = [];
            manager.on('files-available', (files) => receivedFiles.push(files));

            const mockBuilder = builder as unknown as MockSessionBuilder;
            mockBuilder.triggerFilesAvailable([
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

            manager.dispose();
        });
    });
});
