import { describe, it, expect, beforeEach, vi } from 'vitest';
import { RemoteDesktopService } from './remote-desktop.service';
import { ClipboardService } from './clipboard.service';
import { PublicAPI } from './PublicAPI';
import type { Session } from '../interfaces/Session';
import type { SessionBuilder } from '../interfaces/SessionBuilder';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import type { FileTransferProvider } from '../interfaces/FileTransferProvider';

/**
 * Tests for the enableFileTransfer() integration between RemoteDesktopService
 * and FileTransferProvider.
 *
 * These tests verify that:
 * - enableFileTransfer() accepts a FileTransferProvider and enables clipboard
 * - connect() passes provider extensions to the SessionBuilder
 * - connect() calls setSession() on the provider after connection
 * - PublicAPI composes monitoring suppression into the provider hooks
 */

// Minimal mock SessionBuilder
class MockSessionBuilder {
    extensions: unknown[] = [];

    connect = async (): Promise<Session> => {
        return mockSession as unknown as Session;
    };

    proxyAddress(_address: string): this {
        return this;
    }
    destination(_dest: string): this {
        return this;
    }
    serverDomain(_domain: string): this {
        return this;
    }
    password(_pw: string): this {
        return this;
    }
    authToken(_token: string): this {
        return this;
    }
    username(_name: string): this {
        return this;
    }
    renderCanvas(_canvas: HTMLCanvasElement): this {
        return this;
    }
    setCursorStyleCallback(_cb: unknown): this {
        return this;
    }
    setCursorStyleCallbackContext(_ctx: unknown): this {
        return this;
    }
    desktopSize(_size: unknown): this {
        return this;
    }
    remoteClipboardChangedCallback(_cb: unknown): this {
        return this;
    }
    forceClipboardUpdateCallback(_cb: unknown): this {
        return this;
    }
    canvasResizedCallback(_cb: unknown): this {
        return this;
    }
    extension(ext: unknown): this {
        this.extensions.push(ext);
        return this;
    }
}

// Minimal mock Session
const mockSession = {
    desktopSize: vi.fn().mockReturnValue({ width: 1920, height: 1080 }),
    run: vi.fn().mockResolvedValue({ reason: () => 'test' }),
    invokeExtension: vi.fn(),
    shutdown: vi.fn(),
    releaseAllInputs: vi.fn(),
};

let mockBuilderInstance: MockSessionBuilder;

// Mock RemoteDesktopModule
function createMockModule(): RemoteDesktopModule {
    return {
        SessionBuilder: class {
            constructor() {
                return mockBuilderInstance as unknown as SessionBuilder;
            }
        } as unknown as { new (): SessionBuilder },
        DesktopSize: class {
            constructor(
                public width: number,
                public height: number,
            ) {}
        } as unknown as RemoteDesktopModule['DesktopSize'],
        InputTransaction: class {} as unknown as RemoteDesktopModule['InputTransaction'],
        ClipboardData: class {} as unknown as RemoteDesktopModule['ClipboardData'],
        DeviceEvent: {} as unknown as RemoteDesktopModule['DeviceEvent'],
    };
}

// Mock FileTransferProvider
function createMockProvider(): FileTransferProvider {
    return {
        getBuilderExtensions: vi.fn().mockReturnValue([{ id: 'ext1' }, { id: 'ext2' }]),
        setSession: vi.fn(),
        dispose: vi.fn(),
    };
}

describe('enableFileTransfer integration', () => {
    let service: RemoteDesktopService;
    let mockModule: RemoteDesktopModule;

    beforeEach(() => {
        vi.clearAllMocks();
        mockBuilderInstance = new MockSessionBuilder();
        mockModule = createMockModule();
        service = new RemoteDesktopService(mockModule);
        const canvas = document.createElement('canvas');
        service.setCanvas(canvas);
    });

    it('should accept a FileTransferProvider and return it', () => {
        const provider = createMockProvider();
        const result = service.enableFileTransfer(provider);
        expect(result).toBe(provider);
    });

    describe('connect() with FileTransferProvider', () => {
        it('should pass provider extensions to SessionBuilder', async () => {
            const provider = createMockProvider();
            service.enableFileTransfer(provider);

            await service.connect({
                proxyAddress: 'wss://test',
                destination: 'test:3389',
                serverDomain: '',
                password: 'pass',
                authToken: 'token',
                username: 'user',
                desktopSize: { width: 1920, height: 1080 },
                extensions: [],
            });

            // Provider's extensions should have been registered on builder
            expect(provider.getBuilderExtensions).toHaveBeenCalledTimes(1);
            expect(mockBuilderInstance.extensions).toHaveLength(2);
        });

        it('should call setSession after connect', async () => {
            const provider = createMockProvider();
            service.enableFileTransfer(provider);

            await service.connect({
                proxyAddress: 'wss://test',
                destination: 'test:3389',
                serverDomain: '',
                password: 'pass',
                authToken: 'token',
                username: 'user',
                desktopSize: { width: 1920, height: 1080 },
                extensions: [],
            });

            expect(provider.setSession).toHaveBeenCalledTimes(1);
            expect(provider.setSession).toHaveBeenCalledWith(mockSession);
        });
    });

    describe('connect() without FileTransferProvider', () => {
        it('should not register any file transfer extensions', async () => {
            await service.connect({
                proxyAddress: 'wss://test',
                destination: 'test:3389',
                serverDomain: '',
                password: 'pass',
                authToken: 'token',
                username: 'user',
                desktopSize: { width: 1920, height: 1080 },
                extensions: [],
            });

            // No file transfer extensions
            expect(mockBuilderInstance.extensions).toHaveLength(0);
        });
    });
});

describe('PublicAPI clipboard monitoring suppression', () => {
    let service: RemoteDesktopService;
    let clipboardService: ClipboardService;
    let publicApi: PublicAPI;
    let mockModule: RemoteDesktopModule;

    beforeEach(() => {
        vi.clearAllMocks();
        mockBuilderInstance = new MockSessionBuilder();
        mockModule = createMockModule();
        service = new RemoteDesktopService(mockModule);
        clipboardService = new ClipboardService(service, mockModule);
        publicApi = new PublicAPI(service, clipboardService);
    });

    it('should wire suppressMonitoring/resumeMonitoring into provider hooks', () => {
        const provider = createMockProvider();
        const api = publicApi.getExposedFunctions();
        api.enableFileTransfer(provider);

        const suppressSpy = vi.spyOn(clipboardService, 'suppressMonitoring');
        const resumeSpy = vi.spyOn(clipboardService, 'resumeMonitoring');

        provider.onUploadStarted?.();
        expect(suppressSpy).toHaveBeenCalledTimes(1);

        provider.onUploadFinished?.();
        expect(resumeSpy).toHaveBeenCalledTimes(1);
    });

    it('should compose user-provided hooks with monitoring suppression', () => {
        const userStarted = vi.fn();
        const userFinished = vi.fn();
        const provider = createMockProvider();
        provider.onUploadStarted = userStarted;
        provider.onUploadFinished = userFinished;

        const suppressSpy = vi.spyOn(clipboardService, 'suppressMonitoring');
        const resumeSpy = vi.spyOn(clipboardService, 'resumeMonitoring');

        const api = publicApi.getExposedFunctions();
        api.enableFileTransfer(provider);

        provider.onUploadStarted?.();
        expect(userStarted).toHaveBeenCalledTimes(1);
        expect(suppressSpy).toHaveBeenCalledTimes(1);

        provider.onUploadFinished?.();
        expect(userFinished).toHaveBeenCalledTimes(1);
        expect(resumeSpy).toHaveBeenCalledTimes(1);
    });
});
