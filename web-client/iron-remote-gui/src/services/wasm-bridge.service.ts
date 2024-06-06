import { BehaviorSubject, from, Observable, of, Subject } from 'rxjs';
import init, {
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    ironrdp_init,
    IronRdpError,
    Session,
    SessionBuilder,
    ClipboardTransaction,
    SessionTerminationInfo,
} from '../../../../crates/ironrdp-web/pkg/ironrdp_web';
import { loggingService } from './logging.service';
import { catchError, filter, map } from 'rxjs/operators';
import { scanCode } from '../lib/scancodes';
import { LogType } from '../enums/LogType';
import { OS } from '../enums/OS';
import { ModifierKey } from '../enums/ModifierKey';
import { LockKey } from '../enums/LockKey';
import { SessionEventType } from '../enums/SessionEventType';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import type { ResizeEvent } from '../interfaces/ResizeEvent';
import { ScreenScale } from '../enums/ScreenScale';
import type { MousePosition } from '../interfaces/MousePosition';
import type { SessionEvent, UserIronRdpErrorKind } from '../interfaces/session-event';
import type { DesktopSize as IDesktopSize } from '../interfaces/DesktopSize';

type OnRemoteClipboardChanged = (transaction: ClipboardTransaction) => void;
type OnRemoteReceivedFormatsList = () => void;
type OnForceClipboardUpdate = () => void;

export class WasmBridgeService {
    private _resize: Subject<ResizeEvent> = new Subject<ResizeEvent>();
    private mousePosition: BehaviorSubject<MousePosition> = new BehaviorSubject<MousePosition>({
        x: 0,
        y: 0,
    });
    private changeVisibility: Subject<boolean> = new Subject();
    private sessionEvent: Subject<SessionEvent> = new Subject();
    private scale: BehaviorSubject<ScreenScale> = new BehaviorSubject(ScreenScale.Fit as ScreenScale);
    private canvas?: HTMLCanvasElement;
    private keyboardActive: boolean = false;
    private keyboardUnicodeMode: boolean = false;
    private backendSupportsUnicodeKeyboardShortcuts: boolean | undefined = undefined;
    private onRemoteClipboardChanged?: OnRemoteClipboardChanged;
    private onRemoteReceivedFormatList?: OnRemoteReceivedFormatsList;
    private onForceClipboardUpdate?: OnForceClipboardUpdate;
    private cursorHasOverride: boolean = false;
    private lastCursorStyle: string = 'default';

    resize: Observable<ResizeEvent>;
    session?: Session;
    modifierKeyPressed: ModifierKey[] = [];
    mousePositionObservable: Observable<MousePosition> = this.mousePosition.asObservable();
    changeVisibilityObservable: Observable<boolean> = this.changeVisibility.asObservable();
    sessionObserver: Observable<SessionEvent> = this.sessionEvent.asObservable();
    scaleObserver: Observable<ScreenScale> = this.scale.asObservable();

    constructor() {
        this.resize = this._resize.asObservable();
        loggingService.info('Web bridge initialized.');
    }

    async init(debug: LogType) {
        loggingService.info('Loading wasm file.');
        await init();
        loggingService.info('Initializing IronRDP.');
        ironrdp_init(LogType[debug]);
    }

    /// Callback to set the local clipboard content to data received from the remote.
    setOnRemoteClipboardChanged(callback: OnRemoteClipboardChanged) {
        this.onRemoteClipboardChanged = callback;
    }

    /// Callback which is called when the remote sends a list of supported clipboard formats.
    setOnRemoteReceivedFormatList(callback: OnRemoteReceivedFormatsList) {
        this.onRemoteReceivedFormatList = callback;
    }

    /// Callback which is called when the remote requests a forced clipboard update (e.g. on
    /// clipboard initialization sequence)
    setOnForceClipboardUpdate(callback: OnForceClipboardUpdate) {
        this.onForceClipboardUpdate = callback;
    }

    mouseIn(event: MouseEvent) {
        this.syncModifier(event);
        this.keyboardActive = true;
    }

    mouseOut(_event: MouseEvent) {
        this.keyboardActive = false;
        this.releaseAllInputs();
    }

    sendKeyboardEvent(evt: KeyboardEvent) {
        if (this.keyboardActive) {
            this.sendKeyboard(evt);
        }
    }

    shutdown() {
        this.session?.shutdown();
    }

    mouseButtonState(event: MouseEvent, isDown: boolean, preventDefault: boolean) {
        if (preventDefault) {
            event.preventDefault(); // prevent default behavior (context menu, etc)
        }
        const mouseFnc = isDown ? DeviceEvent.new_mouse_button_pressed : DeviceEvent.new_mouse_button_released;
        this.doTransactionFromDeviceEvents([mouseFnc(event.button)]);
    }

    updateMousePosition(position: MousePosition) {
        if (!this.keyboardActive) {
            this.keyboardActive = true;
        }
        this.doTransactionFromDeviceEvents([DeviceEvent.new_mouse_move(position.x, position.y)]);
        this.mousePosition.next(position);
    }

    connect(
        username: string,
        password: string,
        destination: string,
        proxyAddress: string,
        serverDomain: string,
        authToken: string,
        desktopSize?: IDesktopSize,
        preConnectionBlob?: string,
        kdc_proxy_url?: string,
    ): Observable<NewSessionInfo> {
        const sessionBuilder = SessionBuilder.new();
        sessionBuilder.proxy_address(proxyAddress);
        sessionBuilder.destination(destination);
        sessionBuilder.server_domain(serverDomain);
        sessionBuilder.password(password);
        sessionBuilder.auth_token(authToken);
        sessionBuilder.username(username);
        sessionBuilder.render_canvas(this.canvas!);
        sessionBuilder.set_cursor_style_callback_context(this);
        sessionBuilder.set_cursor_style_callback(this.setCursorStyleCallback);
        sessionBuilder.kdc_proxy_url(kdc_proxy_url);

        if (preConnectionBlob != null) {
            sessionBuilder.pcb(preConnectionBlob);
        }
        if (this.onRemoteClipboardChanged != null) {
            sessionBuilder.remote_clipboard_changed_callback(this.onRemoteClipboardChanged);
        }
        if (this.onRemoteReceivedFormatList != null) {
            sessionBuilder.remote_received_format_list_callback(this.onRemoteReceivedFormatList);
        }
        if (this.onForceClipboardUpdate != null) {
            sessionBuilder.force_clipboard_update_callback(this.onForceClipboardUpdate);
        }

        if (desktopSize != null) {
            sessionBuilder.desktop_size(DesktopSize.new(desktopSize.width, desktopSize.height));
        }

        // Type guard to filter out errors
        function isSession(result: IronRdpError | Session): result is Session {
            return result instanceof Session;
        }

        return from(sessionBuilder.connect()).pipe(
            catchError((err: IronRdpError) => {
                this.raiseSessionEvent({
                    type: SessionEventType.ERROR,
                    data: {
                        backtrace: () => err.backtrace(),
                        kind: () => err.kind() as number as UserIronRdpErrorKind,
                    },
                });
                return of(err);
            }),
            filter(isSession),
            map((session: Session) => {
                from(session.run())
                    .pipe(
                        catchError((err) => {
                            this.setVisibility(false);
                            this.raiseSessionEvent({
                                type: SessionEventType.ERROR,
                                data: err.backtrace(),
                            });
                            this.raiseSessionEvent({
                                type: SessionEventType.TERMINATED,
                                data: 'Session was terminated.',
                            });
                            throw err;
                        }),
                        map((termination_info: SessionTerminationInfo) => {
                            this.setVisibility(false);
                            this.raiseSessionEvent({
                                type: SessionEventType.TERMINATED,
                                data: 'Session was terminated: ' + termination_info.reason() + '.',
                            });
                        }),
                    )
                    .subscribe();
                return session;
            }),
            map((session: Session) => {
                loggingService.info('Session started.');
                this.session = session;
                this._resize.next({
                    desktop_size: session.desktop_size(),
                    session_id: 0,
                });
                this.raiseSessionEvent({
                    type: SessionEventType.STARTED,
                    data: 'Session started',
                });
                return {
                    session_id: 0,
                    initial_desktop_size: session.desktop_size(),
                    websocket_port: 0,
                };
            }),
        );
    }

    sendSpecialCombination(specialCombination: SpecialCombination): void {
        switch (specialCombination) {
            case SpecialCombination.CTRL_ALT_DEL:
                this.ctrlAltDel();
                break;
            case SpecialCombination.META:
                this.sendMeta();
                break;
        }
    }

    mouseWheel(event: WheelEvent) {
        const vertical = event.deltaY !== 0;
        const rotation = vertical ? event.deltaY : event.deltaX;
        this.doTransactionFromDeviceEvents([DeviceEvent.new_wheel_rotations(vertical, -rotation)]);
    }

    setVisibility(state: boolean) {
        this.changeVisibility.next(state);
    }

    setScale(scale: ScreenScale) {
        this.scale.next(scale);
    }

    setCanvas(canvas: HTMLCanvasElement) {
        this.canvas = canvas;
    }

    /// Triggered by the browser when local clipboard is updated. Clipboard backend should
    /// cache the content and send it to the server when it is requested.
    onClipboardChanged(transaction: ClipboardTransaction): Promise<void> {
        const onClipboardChangedPromise = async () => {
            await this.session?.on_clipboard_paste(transaction);
        };
        return onClipboardChangedPromise();
    }

    setKeyboardUnicodeMode(use_unicode: boolean) {
        this.keyboardUnicodeMode = use_unicode;
    }

    setCursorStyleOverride(style: string | null) {
        if (style == null) {
            this.canvas!.style.cursor = this.lastCursorStyle;
            this.cursorHasOverride = false;
        } else {
            this.canvas!.style.cursor = style;
            this.cursorHasOverride = true;
        }
    }

    private releaseAllInputs() {
        this.session?.release_all_inputs();
    }

    private supportsUnicodeKeyboardShortcuts(): boolean {
        // Use cached value to reduce FFI calls
        if (this.backendSupportsUnicodeKeyboardShortcuts !== undefined) {
            return this.backendSupportsUnicodeKeyboardShortcuts;
        }

        if (this.session?.supports_unicode_keyboard_shortcuts) {
            this.backendSupportsUnicodeKeyboardShortcuts = this.session?.supports_unicode_keyboard_shortcuts();
            return this.backendSupportsUnicodeKeyboardShortcuts;
        }

        // By default we use unicode keyboard shortcuts for backends
        return true;
    }

    private sendKeyboard(evt: KeyboardEvent) {
        evt.preventDefault();

        let keyEvent;
        let unicodeEvent;

        if (evt.type === 'keydown') {
            keyEvent = DeviceEvent.new_key_pressed;
            unicodeEvent = DeviceEvent.new_unicode_pressed;
        } else if (evt.type === 'keyup') {
            keyEvent = DeviceEvent.new_key_released;
            unicodeEvent = DeviceEvent.new_unicode_released;
        }

        let sendAsUnicode = true;

        if (!this.supportsUnicodeKeyboardShortcuts()) {
            for (const modifier of ['Alt', 'Control', 'Meta', 'AltGraph', 'OS']) {
                if (evt.getModifierState(modifier)) {
                    sendAsUnicode = false;
                    break;
                }
            }
        }

        const isModifierKey = evt.code in ModifierKey;
        const isLockKey = evt.code in LockKey;

        if (isModifierKey) {
            this.updateModifierKeyState(evt);
        }

        if (isLockKey) {
            this.syncModifier(evt);
        }

        if (!evt.repeat || (!isModifierKey && !isLockKey)) {
            const keyScanCode = scanCode(evt.code, OS.WINDOWS);
            const unknownScanCode = Number.isNaN(keyScanCode);

            if (!this.keyboardUnicodeMode && keyEvent && !unknownScanCode) {
                this.doTransactionFromDeviceEvents([keyEvent(keyScanCode)]);
                return;
            }

            if (this.keyboardUnicodeMode && unicodeEvent && keyEvent) {
                // `Dead` and `Unidentified` keys should be ignored
                if (['Dead', 'Unidentified'].indexOf(evt.key) != -1) {
                    return;
                }

                const keyCode = scanCode(evt.key, OS.WINDOWS);
                const isUnicodeCharacter = Number.isNaN(keyCode) && evt.key.length === 1;

                if (isUnicodeCharacter && sendAsUnicode) {
                    this.doTransactionFromDeviceEvents([unicodeEvent(evt.key)]);
                } else if (!unknownScanCode) {
                    // Use scancode insdead of key code for non-unicode character values
                    this.doTransactionFromDeviceEvents([keyEvent(keyScanCode)]);
                }
                return;
            }
        }
    }

    private setCursorStyleCallback(
        style: string,
        data: string | undefined,
        hotspot_x: number | undefined,
        hotspot_y: number | undefined,
    ) {
        let cssStyle;

        switch (style) {
            case 'hidden': {
                cssStyle = 'none';
                break;
            }
            case 'default': {
                cssStyle = 'default';
                break;
            }
            case 'url': {
                if (data == undefined || hotspot_x == undefined || hotspot_y == undefined) {
                    console.error('Invalid custom cursor parameters.');
                    return;
                }

                // IMPORTANT: We need to make proxy `Image` object to actually load the image and
                // make it usable for CSS property. Without this proxy object, URL will be rejected.
                const image = new Image();
                image.src = data;

                const rounded_hotspot_x = Math.round(hotspot_x);
                const rounded_hotspot_y = Math.round(hotspot_y);

                cssStyle = `url(${data}) ${rounded_hotspot_x} ${rounded_hotspot_y}, default`;

                break;
            }
            default: {
                console.error(`Unsupported cursor style: ${style}.`);
                return;
            }
        }

        this.lastCursorStyle = cssStyle;

        if (!this.cursorHasOverride) {
            this.canvas!.style.cursor = cssStyle;
        }
    }

    private syncModifier(evt: KeyboardEvent | MouseEvent): void {
        const syncCapsLockActive = evt.getModifierState(LockKey.CAPS_LOCK);
        const syncNumsLockActive = evt.getModifierState(LockKey.NUM_LOCK);
        const syncScrollLockActive = evt.getModifierState(LockKey.SCROLL_LOCK);
        const syncKanaModeActive = evt.getModifierState(LockKey.KANA_MODE);

        this.session?.synchronize_lock_keys(
            syncScrollLockActive,
            syncNumsLockActive,
            syncCapsLockActive,
            syncKanaModeActive,
        );
    }

    private raiseSessionEvent(event: SessionEvent) {
        this.sessionEvent.next(event);
    }

    private updateModifierKeyState(evt: KeyboardEvent) {
        const modKey: ModifierKey = ModifierKey[evt.code as keyof typeof ModifierKey];

        if (this.modifierKeyPressed.indexOf(modKey) === -1) {
            this.modifierKeyPressed.push(modKey);
        } else if (evt.type === 'keyup') {
            this.modifierKeyPressed.splice(this.modifierKeyPressed.indexOf(modKey), 1);
        }
    }

    private doTransactionFromDeviceEvents(deviceEvents: DeviceEvent[]) {
        const transaction = InputTransaction.new();
        deviceEvents.forEach((event) => transaction.add_event(event));
        this.session?.apply_inputs(transaction);
    }

    private ctrlAltDel() {
        const ctrl = parseInt('0x001D', 16);
        const alt = parseInt('0x0038', 16);
        const suppr = parseInt('0xE053', 16);

        this.doTransactionFromDeviceEvents([
            DeviceEvent.new_key_pressed(ctrl),
            DeviceEvent.new_key_pressed(alt),
            DeviceEvent.new_key_pressed(suppr),
            DeviceEvent.new_key_released(ctrl),
            DeviceEvent.new_key_released(alt),
            DeviceEvent.new_key_released(suppr),
        ]);
    }

    private sendMeta() {
        const meta = parseInt('0xE05B', 16);

        this.doTransactionFromDeviceEvents([DeviceEvent.new_key_pressed(meta), DeviceEvent.new_key_released(meta)]);
    }
}
