/* tslint:disable */
/* eslint-disable */
/**
* @param {string} log_level
*/
export function ironrdp_init(log_level: string): void;
/**
*/
export enum IronRdpErrorKind {
/**
* Catch-all error kind
*/
  General = 0,
/**
* Incorrect password used
*/
  WrongPassword = 1,
/**
* Unable to login to machine
*/
  LogonFailure = 2,
/**
* Insufficient permission, server denied access
*/
  AccessDenied = 3,
/**
* Something wrong happened when sending or receiving the RDCleanPath message
*/
  RDCleanPath = 4,
/**
* Couldn’t connect to proxy
*/
  ProxyConnect = 5,
}
/**
* Object which represents single clipboard format represented standard MIME type.
*/
export class ClipboardContent {
  free(): void;
/**
* @param {string} mime_type
* @param {string} text
* @returns {ClipboardContent}
*/
  static new_text(mime_type: string, text: string): ClipboardContent;
/**
* @param {string} mime_type
* @param {Uint8Array} binary
* @returns {ClipboardContent}
*/
  static new_binary(mime_type: string, binary: Uint8Array): ClipboardContent;
/**
* @returns {string}
*/
  mime_type(): string;
/**
* @returns {any}
*/
  value(): any;
}
/**
* Object which represents complete clipboard transaction with multiple MIME types.
*/
export class ClipboardTransaction {
  free(): void;
/**
* @returns {ClipboardTransaction}
*/
  static new(): ClipboardTransaction;
/**
* @param {ClipboardContent} content
*/
  add_content(content: ClipboardContent): void;
/**
* @returns {boolean}
*/
  is_empty(): boolean;
/**
* @returns {Array<any>}
*/
  content(): Array<any>;
}
/**
*/
export class DesktopSize {
  free(): void;
/**
* @param {number} width
* @param {number} height
* @returns {DesktopSize}
*/
  static new(width: number, height: number): DesktopSize;
/**
*/
  height: number;
/**
*/
  width: number;
}
/**
*/
export class DeviceEvent {
  free(): void;
/**
* @param {number} button
* @returns {DeviceEvent}
*/
  static new_mouse_button_pressed(button: number): DeviceEvent;
/**
* @param {number} button
* @returns {DeviceEvent}
*/
  static new_mouse_button_released(button: number): DeviceEvent;
/**
* @param {number} x
* @param {number} y
* @returns {DeviceEvent}
*/
  static new_mouse_move(x: number, y: number): DeviceEvent;
/**
* @param {boolean} vertical
* @param {number} rotation_units
* @returns {DeviceEvent}
*/
  static new_wheel_rotations(vertical: boolean, rotation_units: number): DeviceEvent;
/**
* @param {number} scancode
* @returns {DeviceEvent}
*/
  static new_key_pressed(scancode: number): DeviceEvent;
/**
* @param {number} scancode
* @returns {DeviceEvent}
*/
  static new_key_released(scancode: number): DeviceEvent;
/**
* @param {string} character
* @returns {DeviceEvent}
*/
  static new_unicode_pressed(character: string): DeviceEvent;
/**
* @param {string} character
* @returns {DeviceEvent}
*/
  static new_unicode_released(character: string): DeviceEvent;
}
/**
*/
export class InputTransaction {
  free(): void;
/**
* @returns {InputTransaction}
*/
  static new(): InputTransaction;
/**
* @param {DeviceEvent} event
*/
  add_event(event: DeviceEvent): void;
}
/**
*/
export class IronRdpError {
  free(): void;
/**
* @returns {string}
*/
  backtrace(): string;
/**
* @returns {IronRdpErrorKind}
*/
  kind(): IronRdpErrorKind;
}
/**
*/
export class Session {
  free(): void;
/**
* @returns {Promise<SessionTerminationInfo>}
*/
  run(): Promise<SessionTerminationInfo>;
/**
* @returns {DesktopSize}
*/
  desktop_size(): DesktopSize;
/**
* @param {InputTransaction} transaction
*/
  apply_inputs(transaction: InputTransaction): void;
/**
*/
  release_all_inputs(): void;
/**
* @param {boolean} _scroll_lock
* @param {boolean} _num_lock
* @param {boolean} _caps_lock
* @param {boolean} _kana_lock
*/
  synchronize_lock_keys(_scroll_lock: boolean, _num_lock: boolean, _caps_lock: boolean, _kana_lock: boolean): void;
/**
*/
  shutdown(): void;
/**
* @param {ClipboardTransaction} _content
* @returns {Promise<void>}
*/
  on_clipboard_paste(_content: ClipboardTransaction): Promise<void>;
/**
* @returns {boolean}
*/
  supports_unicode_keyboard_shortcuts(): boolean;
}
/**
*/
export class SessionBuilder {
  free(): void;
/**
* @returns {SessionBuilder}
*/
  static new(): SessionBuilder;
/**
* Required
* @param {string} username
* @returns {SessionBuilder}
*/
  username(username: string): SessionBuilder;
/**
* Required
* @param {string} destination
* @returns {SessionBuilder}
*/
  destination(destination: string): SessionBuilder;
/**
* Optional
* @param {string} server_domain
* @returns {SessionBuilder}
*/
  server_domain(server_domain: string): SessionBuilder;
/**
* Required
* @param {string} password
* @returns {SessionBuilder}
*/
  password(password: string): SessionBuilder;
/**
* Required
* @param {string} address
* @returns {SessionBuilder}
*/
  proxy_address(address: string): SessionBuilder;
/**
* Required
* @param {string} token
* @returns {SessionBuilder}
*/
  auth_token(token: string): SessionBuilder;
/**
* Optional
* @param {string} arg0
* @returns {SessionBuilder}
*/
  pcb(arg0: string): SessionBuilder;
/**
* Optional
* @param {string | undefined} [kdc_proxy_url]
* @returns {SessionBuilder}
*/
  kdc_proxy_url(kdc_proxy_url?: string): SessionBuilder;
/**
* Optional
* @param {DesktopSize} desktop_size
* @returns {SessionBuilder}
*/
  desktop_size(desktop_size: DesktopSize): SessionBuilder;
/**
* Required
* @param {HTMLCanvasElement} canvas
* @returns {SessionBuilder}
*/
  render_canvas(canvas: HTMLCanvasElement): SessionBuilder;
/**
* Optional
*
* # Callback signature:
* ```typescript
* function callback(
*     cursor_kind: string,
*     cursor_data: string | undefined,
*     hotspot_x: number | undefined,
*     hotspot_y: number | undefined
* ): void
* ```
*
* # Cursor kinds:
* - `default` (default system cursor); other arguments are `UNDEFINED`
* - `none` (hide cursor); other arguments are `UNDEFINED`
* - `url` (custom cursor data URL); `cursor_data` contains the data URL with Base64-encoded
*   cursor bitmap; `hotspot_x` and `hotspot_y` are set to the cursor hotspot coordinates.
* @param {Function} callback
* @returns {SessionBuilder}
*/
  set_cursor_style_callback(callback: Function): SessionBuilder;
/**
* Optional
* @param {any} context
* @returns {SessionBuilder}
*/
  set_cursor_style_callback_context(context: any): SessionBuilder;
/**
* Optional
* @param {Function} callback
* @returns {SessionBuilder}
*/
  remote_clipboard_changed_callback(callback: Function): SessionBuilder;
/**
* Optional
* @param {Function} callback
* @returns {SessionBuilder}
*/
  remote_received_format_list_callback(callback: Function): SessionBuilder;
/**
* Optional
* @param {Function} callback
* @returns {SessionBuilder}
*/
  force_clipboard_update_callback(callback: Function): SessionBuilder;
/**
* @returns {Promise<Session>}
*/
  connect(): Promise<Session>;
}
/**
*/
export class SessionTerminationInfo {
  free(): void;
/**
* @returns {string}
*/
  reason(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_sessionbuilder_free: (a: number) => void;
  readonly sessionbuilder_new: () => number;
  readonly sessionbuilder_username: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_destination: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_server_domain: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_password: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_proxy_address: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_auth_token: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_pcb: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_kdc_proxy_url: (a: number, b: number, c: number) => number;
  readonly sessionbuilder_desktop_size: (a: number, b: number) => number;
  readonly sessionbuilder_render_canvas: (a: number, b: number) => number;
  readonly sessionbuilder_set_cursor_style_callback: (a: number, b: number) => number;
  readonly sessionbuilder_set_cursor_style_callback_context: (a: number, b: number) => number;
  readonly sessionbuilder_remote_clipboard_changed_callback: (a: number, b: number) => number;
  readonly sessionbuilder_remote_received_format_list_callback: (a: number, b: number) => number;
  readonly sessionbuilder_force_clipboard_update_callback: (a: number, b: number) => number;
  readonly sessionbuilder_connect: (a: number) => number;
  readonly sessionterminationinfo_reason: (a: number, b: number) => void;
  readonly __wbg_session_free: (a: number) => void;
  readonly session_run: (a: number) => number;
  readonly session_desktop_size: (a: number) => number;
  readonly session_apply_inputs: (a: number, b: number, c: number) => void;
  readonly session_release_all_inputs: (a: number, b: number) => void;
  readonly session_synchronize_lock_keys: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly session_shutdown: (a: number, b: number) => void;
  readonly session_on_clipboard_paste: (a: number, b: number) => number;
  readonly session_supports_unicode_keyboard_shortcuts: (a: number) => number;
  readonly __wbg_sessionterminationinfo_free: (a: number) => void;
  readonly ironrdp_init: (a: number, b: number) => void;
  readonly __wbg_desktopsize_free: (a: number) => void;
  readonly __wbg_get_desktopsize_width: (a: number) => number;
  readonly __wbg_set_desktopsize_width: (a: number, b: number) => void;
  readonly __wbg_get_desktopsize_height: (a: number) => number;
  readonly __wbg_set_desktopsize_height: (a: number, b: number) => void;
  readonly desktopsize_new: (a: number, b: number) => number;
  readonly clipboardtransaction_new: () => number;
  readonly clipboardtransaction_add_content: (a: number, b: number) => void;
  readonly clipboardtransaction_is_empty: (a: number) => number;
  readonly clipboardtransaction_content: (a: number) => number;
  readonly clipboardcontent_new_binary: (a: number, b: number, c: number, d: number) => number;
  readonly clipboardcontent_mime_type: (a: number, b: number) => void;
  readonly clipboardcontent_value: (a: number) => number;
  readonly clipboardcontent_new_text: (a: number, b: number, c: number, d: number) => number;
  readonly __wbg_clipboardtransaction_free: (a: number) => void;
  readonly __wbg_clipboardcontent_free: (a: number) => void;
  readonly __wbg_ironrdperror_free: (a: number) => void;
  readonly ironrdperror_backtrace: (a: number, b: number) => void;
  readonly ironrdperror_kind: (a: number) => number;
  readonly __wbg_deviceevent_free: (a: number) => void;
  readonly deviceevent_new_mouse_button_pressed: (a: number) => number;
  readonly deviceevent_new_mouse_button_released: (a: number) => number;
  readonly deviceevent_new_mouse_move: (a: number, b: number) => number;
  readonly deviceevent_new_wheel_rotations: (a: number, b: number) => number;
  readonly deviceevent_new_key_pressed: (a: number) => number;
  readonly deviceevent_new_key_released: (a: number) => number;
  readonly deviceevent_new_unicode_pressed: (a: number) => number;
  readonly deviceevent_new_unicode_released: (a: number) => number;
  readonly __wbg_inputtransaction_free: (a: number) => void;
  readonly inputtransaction_new: () => number;
  readonly inputtransaction_add_event: (a: number, b: number) => void;
  readonly ring_core_0_17_7_bn_mul_mont: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_export_2: WebAssembly.Table;
  readonly wasm_bindgen__convert__closures__invoke0_mut__h472a2eec1eb63f16: (a: number, b: number) => void;
  readonly _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h52ac3e07abf19181: (a: number, b: number, c: number) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h567eb7933fc125a1: (a: number, b: number) => void;
  readonly _dyn_core__ops__function__FnMut__A____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h8cf2451f34e79e5d: (a: number, b: number, c: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly wasm_bindgen__convert__closures__invoke2_mut__h794127c7cda2640f: (a: number, b: number, c: number, d: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {SyncInitInput} module
*
* @returns {InitOutput}
*/
export function initSync(module: SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {InitInput | Promise<InitInput>} module_or_path
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: InitInput | Promise<InitInput>): Promise<InitOutput>;
