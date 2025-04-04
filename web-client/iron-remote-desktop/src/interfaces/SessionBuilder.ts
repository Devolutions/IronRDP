import type { Session } from './Session';
import type { DesktopSize } from './DesktopSize';
import type { ClipboardTransaction } from './ClipboardTransaction';
export interface SessionBuilder {
    construct(): SessionBuilder;
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
    server_domain(server_domain: string): SessionBuilder;
    /**
     * Required
     */
    password(password: string): SessionBuilder;
    /**
     * Required
     */
    proxy_address(address: string): SessionBuilder;
    /**
     * Required
     */
    auth_token(token: string): SessionBuilder;
    /**
     * Optional
     */
    desktop_size(desktop_size: DesktopSize): SessionBuilder;
    /**
     * Optional
     */
    render_canvas(canvas: HTMLCanvasElement): SessionBuilder;
    /**
     * Required.
     *
     * # Cursor kinds:
     * - `default` (default system cursor); other arguments are `UNDEFINED`
     * - `none` (hide cursor); other arguments are `UNDEFINED`
     * - `url` (custom cursor data URL); `cursor_data` contains the data URL with Base64-encoded
     *   cursor bitmap; `hotspot_x` and `hotspot_y` are set to the cursor hotspot coordinates.
     */
    set_cursor_style_callback(callback: SetCursorStyleCallback): SessionBuilder;
    /**
     * Required.
     */
    set_cursor_style_callback_context(context: unknown): SessionBuilder;
    /**
     * Optional
     */
    remote_clipboard_changed_callback(callback: RemoteClipboardChangedCallback): SessionBuilder;
    /**
     * Optional
     */
    remote_received_format_list_callback(callback: RemoteReceiveForwardListCallback): SessionBuilder;
    /**
     * Optional
     */
    force_clipboard_update_callback(callback: ForceClipboardUpdateCallback): SessionBuilder;
    extension(value: unknown): SessionBuilder;
    // eslint-disable-next-line @typescript-eslint/no-unsafe-function-type
    extension_call(_ident: string, _call: Function): SessionBuilder;
    connect(): Promise<Session>;
}

interface SetCursorStyleCallback {
    (
        cursor_kind: string,
        cursor_data: string | undefined,
        hotspot_x: number | undefined,
        hotspot_y: number | undefined,
    ): void;
}

interface RemoteClipboardChangedCallback {
    (transaction: ClipboardTransaction): void;
}

interface RemoteReceiveForwardListCallback {
    (): void;
}

interface ForceClipboardUpdateCallback {
    (): void;
}
