import type { Session } from './Session';
import type { DesktopSize } from './DesktopSize';
import type { ClipboardData } from './ClipboardData';
import type { FileInfo, FileContentsRequest, FileContentsResponse } from './FileTransfer';

export interface SessionBuilder {
    /**
     * Required
     */
    username(username: string): SessionBuilder;
    /**
     * Required
     */
    destination(destination: string): SessionBuilder;
    /**
     * Optional
     */
    serverDomain(serverDomain: string): SessionBuilder;
    /**
     * Required
     */
    password(password: string): SessionBuilder;
    /**
     * Required
     */
    proxyAddress(address: string): SessionBuilder;
    /**
     * Required
     */
    authToken(token: string): SessionBuilder;
    /**
     * Optional
     */
    desktopSize(desktopSize: DesktopSize): SessionBuilder;
    /**
     * Optional
     */
    renderCanvas(canvas: HTMLCanvasElement): SessionBuilder;
    /**
     * Required.
     *
     * # Cursor kinds:
     * - `default` (default system cursor); other arguments are `UNDEFINED`
     * - `none` (hide cursor); other arguments are `UNDEFINED`
     * - `url` (custom cursor data URL); `cursor_data` contains the data URL with Base64-encoded
     *   cursor bitmap; `hotspot_x` and `hotspot_y` are set to the cursor hotspot coordinates.
     */
    setCursorStyleCallback(callback: SetCursorStyleCallback): SessionBuilder;
    /**
     * Required.
     */
    setCursorStyleCallbackContext(context: unknown): SessionBuilder;
    /**
     * Optional
     */
    remoteClipboardChangedCallback(callback: RemoteClipboardChangedCallback): SessionBuilder;
    /**
     * Optional
     */
    forceClipboardUpdateCallback(callback: ForceClipboardUpdateCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when remote copies files. The array contains metadata for each file available
     * for download. The optional clipDataId is the clipboard lock that was
     * acquired automatically when the file list was received. Pass it to
     * requestFileContents() to associate downloads with the lock.
     */
    filesAvailableCallback(callback: FilesAvailableCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when remote requests file contents from client (upload). The callback should
     * read the requested file chunk and respond via submitFileContents().
     */
    fileContentsRequestCallback(callback: FileContentsRequestCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when remote sends file contents to client (download). This is the response
     * to a previous file contents request initiated by the client.
     */
    fileContentsResponseCallback(callback: FileContentsResponseCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when remote locks their clipboard for file transfer. The dataId associates
     * subsequent FileContentsRequest/Response cycles.
     */
    lockCallback(callback: LockCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when remote unlocks their clipboard after file transfer completes or
     * when new clipboard content is copied (auto-unlock per MS-RDPECLIP spec).
     */
    unlockCallback(callback: UnlockCallback): SessionBuilder;
    /**
     * Optional
     *
     * Called when client-side clipboard locks expire due to inactivity timeout.
     * This notifies the application when automatic cleanup has removed locks that
     * have been inactive for too long or exceeded maximum lifetime. The locks have
     * already been unlocked when this callback is invoked.
     *
     * Use this callback to:
     * - Clear any references to expired lock IDs
     * - Abort associated file transfers
     * - Update UI to reflect lock expiration
     */
    locksExpiredCallback(callback: LocksExpiredCallback): SessionBuilder;
    /**
     * Optional
     */
    canvasResizedCallback(callback: CanvasResizedCallback): SessionBuilder;
    extension(value: unknown): SessionBuilder;
    connect(): Promise<Session>;
}

interface SetCursorStyleCallback {
    (
        cursorKind: string,
        cursorData: string | undefined,
        hotspotX: number | undefined,
        hotspotY: number | undefined,
    ): void;
}

interface RemoteClipboardChangedCallback {
    (data: ClipboardData): void;
}

interface ForceClipboardUpdateCallback {
    (): void;
}

interface FilesAvailableCallback {
    (files: FileInfo[], clipDataId?: number): void;
}

interface FileContentsRequestCallback {
    (request: FileContentsRequest): void;
}

interface FileContentsResponseCallback {
    (response: FileContentsResponse): void;
}

interface LockCallback {
    (dataId: number): void;
}

interface UnlockCallback {
    (dataId: number): void;
}

interface LocksExpiredCallback {
    (clipDataIds: Uint32Array): void;
}

interface CanvasResizedCallback {
    (): void;
}
