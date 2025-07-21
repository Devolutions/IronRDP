import type { Session } from './Session';
import type { DesktopSize } from './DesktopSize';
import type { ClipboardData } from './ClipboardData';

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
    remoteReceivedFormatListCallback(callback: RemoteReceiveForwardListCallback): SessionBuilder;
    /**
     * Optional
     */
    forceClipboardUpdateCallback(callback: ForceClipboardUpdateCallback): SessionBuilder;
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

interface RemoteReceiveForwardListCallback {
    (): void;
}

interface ForceClipboardUpdateCallback {
    (): void;
}

interface CanvasResizedCallback {
    (): void;
}
