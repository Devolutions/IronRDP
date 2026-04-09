import type { Extension } from './Extension';
import type { Session } from './Session';

/**
 * Protocol-agnostic interface for file transfer providers.
 *
 * Implementations live in protocol-specific packages (e.g., `RdpFileTransferProvider`
 * in `iron-remote-desktop-rdp`) and are injected into the web component via
 * `enableFileTransfer()`.
 */
export interface FileTransferProvider {
    /** Extensions to register on the SessionBuilder before connect(). */
    getBuilderExtensions(): Extension[];

    /** Called after connect() with the live session. */
    setSession(session: Session): void;

    /** Called when an upload begins (use for monitoring suppression). */
    onUploadStarted?: () => void;

    /** Called when an upload ends or is abandoned. */
    onUploadFinished?: () => void;

    /** Clean up resources. */
    dispose(): void;
}
