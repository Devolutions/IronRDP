import type { InputTransaction } from './InputTransaction';
import type { DesktopSize } from './DesktopSize';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardData } from './ClipboardData';

export interface Session {
    run(): Promise<SessionTerminationInfo>;
    desktopSize(): DesktopSize;
    applyInputs(transaction: InputTransaction): void;
    releaseAllInputs(): void;
    synchronizeLockKeys(scrollLock: boolean, numLock: boolean, capsLock: boolean, kanaLock: boolean): void;
    extensionCall(value: unknown): unknown;
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
}
