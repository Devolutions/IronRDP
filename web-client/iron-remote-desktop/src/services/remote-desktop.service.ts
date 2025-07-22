import { loggingService } from './logging.service';
import { scanCode } from '../lib/scancodes';
import { ModifierKey } from '../enums/ModifierKey';
import { LockKey } from '../enums/LockKey';
import { SessionEventType } from '../enums/SessionEventType';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import type { ResizeEvent } from '../interfaces/ResizeEvent';
import { ScreenScale } from '../enums/ScreenScale';
import type { MousePosition } from '../interfaces/MousePosition';
import type { IronError, IronErrorKind, SessionEvent } from '../interfaces/session-event';
import type { ClipboardData } from '../interfaces/ClipboardData';
import type { Session } from '../interfaces/Session';
import type { DeviceEvent } from '../interfaces/DeviceEvent';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import { ConfigBuilder } from './ConfigBuilder';
import type { Config } from './Config';
import type { Extension } from '../interfaces/Extension';
import { Observable } from '../lib/Observable';
import type { SessionTerminationInfo } from '../interfaces/SessionTerminationInfo';
import type { ConfigParser } from '../interfaces/ConfigParser';

type OnRemoteClipboardChanged = (data: ClipboardData) => void;
type OnRemoteReceivedFormatsList = () => void;
type OnForceClipboardUpdate = () => void;
type OnCanvasResized = () => void;

export class RemoteDesktopService {
    private module: RemoteDesktopModule;
    private canvas?: HTMLCanvasElement;
    private keyboardUnicodeMode: boolean = false;
    private backendSupportsUnicodeKeyboardShortcuts: boolean | undefined = undefined;
    private onRemoteClipboardChanged?: OnRemoteClipboardChanged;
    private onRemoteReceivedFormatList?: OnRemoteReceivedFormatsList;
    private onForceClipboardUpdate?: OnForceClipboardUpdate;
    private onCanvasResized?: OnCanvasResized;
    private cursorHasOverride: boolean = false;
    private lastCursorStyle: string = 'default';
    private enableClipboard: boolean = true;

    resizeObservable: Observable<ResizeEvent> = new Observable();

    session?: Session;
    modifierKeyPressed: ModifierKey[] = [];

    mousePositionObservable: Observable<MousePosition> = new Observable();
    changeVisibilityObservable: Observable<boolean> = new Observable();
    sessionEventObservable: Observable<SessionEvent> = new Observable();
    scaleObservable: Observable<ScreenScale> = new Observable();

    dynamicResizeObservable: Observable<{ width: number; height: number }> = new Observable();

    constructor(module: RemoteDesktopModule) {
        this.module = module;
        loggingService.info('Web bridge initialized.');
    }

    // If set to false, the clipboard will not be enabled and the callbacks will not be registered to the Rust side
    setEnableClipboard(enable: boolean) {
        this.enableClipboard = enable;
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

    /// Callback which is called when the canvas is resized.
    setOnCanvasResized(callback: OnCanvasResized) {
        this.onCanvasResized = callback;
    }

    mouseIn(event: MouseEvent) {
        this.syncModifier(event);
    }

    mouseOut(_event: MouseEvent) {
        this.releaseAllInputs();
    }

    sendKeyboardEvent(evt: KeyboardEvent) {
        this.sendKeyboard(evt);
    }

    shutdown() {
        this.session?.shutdown();
    }

    mouseButtonState(event: MouseEvent, isDown: boolean, preventDefault: boolean) {
        if (preventDefault) {
            event.preventDefault(); // prevent default behavior (context menu, etc)
        }
        const mouseFnc = isDown
            ? this.module.DeviceEvent.mouseButtonPressed
            : this.module.DeviceEvent.mouseButtonReleased;
        this.doTransactionFromDeviceEvents([mouseFnc(event.button)]);
    }

    updateMousePosition(position: MousePosition) {
        this.doTransactionFromDeviceEvents([this.module.DeviceEvent.mouseMove(position.x, position.y)]);
        this.mousePositionObservable.publish(position);
    }

    configBuilder(): ConfigBuilder {
        return new ConfigBuilder();
    }

    configParser(config: string): ConfigParser {
        return new this.module.ConfigParser(config);
    }

    async connect(config: Config): Promise<NewSessionInfo> {
        const sessionBuilder = new this.module.SessionBuilder();

        sessionBuilder.proxyAddress(config.proxyAddress);
        sessionBuilder.destination(config.destination);
        sessionBuilder.serverDomain(config.serverDomain);
        sessionBuilder.password(config.password);
        sessionBuilder.authToken(config.authToken);
        sessionBuilder.username(config.username);
        sessionBuilder.renderCanvas(this.canvas!);
        sessionBuilder.setCursorStyleCallbackContext(this);
        sessionBuilder.setCursorStyleCallback(this.setCursorStyleCallback);

        config.extensions.forEach((extension) => {
            sessionBuilder.extension(extension);
        });

        if (this.onRemoteClipboardChanged != null && this.enableClipboard) {
            sessionBuilder.remoteClipboardChangedCallback(this.onRemoteClipboardChanged);
        }
        if (this.onRemoteReceivedFormatList != null && this.enableClipboard) {
            sessionBuilder.remoteReceivedFormatListCallback(this.onRemoteReceivedFormatList);
        }
        if (this.onForceClipboardUpdate != null && this.enableClipboard) {
            sessionBuilder.forceClipboardUpdateCallback(this.onForceClipboardUpdate);
        }
        if (this.onCanvasResized != null) {
            sessionBuilder.canvasResizedCallback(this.onCanvasResized);
        }

        if (config.desktopSize != null) {
            sessionBuilder.desktopSize(
                new this.module.DesktopSize(config.desktopSize.width, config.desktopSize.height),
            );
        }

        const session = await sessionBuilder.connect().catch((err: IronError) => {
            this.raiseSessionEvent({
                type: SessionEventType.ERROR,
                data: {
                    backtrace: () => err.backtrace(),
                    kind: () => err.kind() as number as IronErrorKind,
                },
            });
            throw new Error('could not connect to the session');
        });

        this.run(session);

        loggingService.info('Session started.');

        this.session = session;

        this.resizeObservable.publish({
            desktopSize: session.desktopSize(),
            sessionId: 0,
        });
        this.raiseSessionEvent({
            type: SessionEventType.STARTED,
            data: 'Session started',
        });

        return {
            sessionId: 0,
            initialDesktopSize: session.desktopSize(),
            websocketPort: 0,
        };
    }

    run(session: Session) {
        session
            .run()
            .then((terminationInfo: SessionTerminationInfo) => {
                this.setVisibility(false);
                this.raiseSessionEvent({
                    type: SessionEventType.TERMINATED,
                    data: 'Session was terminated: ' + terminationInfo.reason() + '.',
                });
            })
            .catch((err: IronError) => {
                this.setVisibility(false);

                this.raiseSessionEvent({
                    type: SessionEventType.TERMINATED,
                    data: 'Session was terminated with an error: ' + err.backtrace() + '.',
                });
            });
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
        this.doTransactionFromDeviceEvents([this.module.DeviceEvent.wheelRotations(vertical, -rotation)]);
    }

    setVisibility(state: boolean) {
        this.changeVisibilityObservable.publish(state);
    }

    setScale(scale: ScreenScale) {
        this.scaleObservable.publish(scale);
    }

    setCanvas(canvas: HTMLCanvasElement) {
        this.canvas = canvas;
    }

    resizeDynamic(width: number, height: number, scale?: number) {
        this.dynamicResizeObservable.publish({ width, height });
        this.session?.resize(width, height, scale);
    }

    /// Triggered by the browser when local clipboard is updated. Clipboard backend should
    /// cache the content and send it to the server when it is requested.
    onClipboardChanged(data: ClipboardData): Promise<void> {
        const onClipboardChangedPromise = async () => {
            await this.session?.onClipboardPaste(data);
        };
        return onClipboardChangedPromise();
    }

    onClipboardChangedEmpty(): Promise<void> {
        const onClipboardChangedPromise = async () => {
            await this.session?.onClipboardPaste(new this.module.ClipboardData());
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

    invokeExtension(ext: Extension) {
        this.session?.invokeExtension(ext);
    }

    private releaseAllInputs() {
        this.session?.releaseAllInputs();
    }

    private supportsUnicodeKeyboardShortcuts(): boolean {
        // Use cached value to reduce FFI calls
        if (this.backendSupportsUnicodeKeyboardShortcuts !== undefined) {
            return this.backendSupportsUnicodeKeyboardShortcuts;
        }

        if (this.session?.supportsUnicodeKeyboardShortcuts) {
            this.backendSupportsUnicodeKeyboardShortcuts = this.session?.supportsUnicodeKeyboardShortcuts();
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
            keyEvent = this.module.DeviceEvent.keyPressed;
            unicodeEvent = this.module.DeviceEvent.unicodePressed;
        } else if (evt.type === 'keyup') {
            keyEvent = this.module.DeviceEvent.keyReleased;
            unicodeEvent = this.module.DeviceEvent.unicodeReleased;
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
            const keyScanCode = scanCode(evt.code);
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

                const keyCode = scanCode(evt.key);
                const isUnicodeCharacter = Number.isNaN(keyCode) && evt.key.length === 1;

                if (isUnicodeCharacter && sendAsUnicode) {
                    this.doTransactionFromDeviceEvents([unicodeEvent(evt.key)]);
                } else if (!unknownScanCode) {
                    // Use scancode instead of key code for non-unicode character values
                    this.doTransactionFromDeviceEvents([keyEvent(keyScanCode)]);
                }
                return;
            }
        }
    }

    private setCursorStyleCallback(
        style: string,
        data: string | undefined,
        hotspotX: number | undefined,
        hotspotY: number | undefined,
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
                if (data == undefined || hotspotX == undefined || hotspotY == undefined) {
                    console.error('Invalid custom cursor parameters.');
                    return;
                }

                // IMPORTANT: We need to make proxy `Image` object to actually load the image and
                // make it usable for CSS property. Without this proxy object, URL will be rejected.
                const image = new Image();
                image.src = data;

                const rounded_hotspot_x = Math.round(hotspotX);
                const rounded_hotspot_y = Math.round(hotspotY);

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

        this.session?.synchronizeLockKeys(
            syncScrollLockActive,
            syncNumsLockActive,
            syncCapsLockActive,
            syncKanaModeActive,
        );
    }

    private raiseSessionEvent(event: SessionEvent) {
        this.sessionEventObservable.publish(event);
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
        const transaction = new this.module.InputTransaction();
        deviceEvents.forEach((event) => transaction.addEvent(event));
        this.session?.applyInputs(transaction);
    }

    private ctrlAltDel() {
        const ctrl = parseInt('0x001D', 16);
        const alt = parseInt('0x0038', 16);
        const suppr = parseInt('0xE053', 16);

        this.doTransactionFromDeviceEvents([
            this.module.DeviceEvent.keyPressed(ctrl),
            this.module.DeviceEvent.keyPressed(alt),
            this.module.DeviceEvent.keyPressed(suppr),
            this.module.DeviceEvent.keyReleased(ctrl),
            this.module.DeviceEvent.keyReleased(alt),
            this.module.DeviceEvent.keyReleased(suppr),
        ]);
    }

    private sendMeta() {
        const meta = parseInt('0xE05B', 16);

        this.doTransactionFromDeviceEvents([
            this.module.DeviceEvent.keyPressed(meta),
            this.module.DeviceEvent.keyReleased(meta),
        ]);
    }
}
