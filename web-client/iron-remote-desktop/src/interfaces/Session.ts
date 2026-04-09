import type { InputTransaction } from './InputTransaction';
import type { DesktopSize } from './DesktopSize';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardData } from './ClipboardData';
import type { Extension } from './Extension';

export interface Session {
    run(): Promise<SessionTerminationInfo>;
    desktopSize(): DesktopSize;
    applyInputs(transaction: InputTransaction): void;
    releaseAllInputs(): void;
    synchronizeLockKeys(scrollLock: boolean, numLock: boolean, capsLock: boolean, kanaLock: boolean): void;
    shutdown(): void;
    onClipboardPaste(data: ClipboardData): Promise<void>;
    resize(
        width: number,
        height: number,
        scaleFactor?: number | null,
        physicalWidth?: number | null,
        physicalHeight?: number | null,
    ): void;
    supportsUnicodeKeyboardShortcuts(): boolean;

    /**
     * Invoke a protocol-specific extension at runtime.
     *
     * File transfer operations (requestFileContents, submitFileContents,
     * initiateFileCopy) are protocol-specific and routed through this
     * method rather than living on Session directly.
     */
    invokeExtension(ext: Extension): unknown;
}
