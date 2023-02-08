import type {NewSessionInfo, ResizeEvent, ServerBridgeService} from './server-bridge.service';
import {MouseButton, MouseButtonState} from './server-bridge.service';
import {from, Observable, of, Subject} from 'rxjs';
import init, {DeviceEvent, InputTransaction, ironrdp_init, Session, SessionBuilder} from "../../../ffi/wasm/pkg/ironrdp";
import {loggingService} from "./logging.service";
import {catchError, filter, map} from "rxjs/operators";
import {userInteractionService} from "./user-interaction-service";
import {scanCode} from '../lib/scancodes';
import {LogType} from '../enums/LogType';
import {OS} from '../enums/OS';

enum ModifierKey {
    CTRL_LEFT = "ControlLeft",
    SHIFT_LEFT = "ShiftLeft",
    SHIFT_RIGHT = "ShiftRight",
    ALT_LEFT = "AltLeft",
    CAPS_LOCK = "CapsLock",
    CTRL_RIGHT = "ControlRight",
    ALT_RIGHT = "AltRight",
    "ControlLeft" = CTRL_LEFT,
    "ShiftLeft" = SHIFT_LEFT,
    "ShiftRight" = SHIFT_RIGHT,
    "AltLeft" = ALT_LEFT,
    "CapsLock" = CAPS_LOCK,
    "ControlRight" = CTRL_RIGHT,
    "AltRight" = ALT_RIGHT,
};

export class WasmBridgeService implements ServerBridgeService {
    private _resize: Subject<ResizeEvent> = new Subject<any>();
    private _updateImage: Subject<any> = new Subject<any>();

    resize: Observable<ResizeEvent>;
    updateImage: Observable<any>;

    session?: Session;

    modifierKeyPressed: ModifierKey[] = [];

    constructor() {
        this.resize = this._resize.asObservable();
        this.updateImage = this._updateImage.asObservable();
        loggingService.info('Web bridge initialized.');
    }

    async init(debug: LogType) {
        loggingService.info('Loading wasm file.');
        await init();
        loggingService.info('Initializing IronRDP.');
        ironrdp_init(LogType[debug]);
    }

    releaseAllInputs() {
        this.session?.release_all_inputs();
    }

    mouseButtonState(mouse_button: MouseButton, state: MouseButtonState) {
        let mouseFnc = state === MouseButtonState.MOUSE_DOWN ? DeviceEvent.new_mouse_button_pressed : DeviceEvent.new_mouse_button_released;
        this.doTransactionFromDeviceEvents([mouseFnc(mouse_button)]);
    }

    updateMousePosition(mouse_x: number, mouse_y: number) {
        this.doTransactionFromDeviceEvents([DeviceEvent.new_mouse_move(mouse_x, mouse_y)]);
    }

    sendKeyboard(evt: KeyboardEvent) {
        evt.preventDefault();

        let keyEvent;

        if (evt.type === 'keydown') {
            keyEvent = DeviceEvent.new_key_pressed;
        } else if (evt.type === 'keyup') {
            keyEvent = DeviceEvent.new_key_released;
        }

        if (keyEvent) {
            const deviceEvents = [];
            const scancode = scanCode(evt.code, OS.WINDOWS);

            if (ModifierKey[evt.code]) {
                if (this.modifierKeyPressed.indexOf(ModifierKey[evt.code]) === -1) {
                    this.modifierKeyPressed.push(ModifierKey[evt.code]);
                    deviceEvents.push(keyEvent(scancode));
                } else if (evt.type === 'keyup') {
                    this.modifierKeyPressed.splice(this.modifierKeyPressed.indexOf(ModifierKey[evt.code]), 1);
                    deviceEvents.push(keyEvent(scancode));
                }
            }

            if (!ModifierKey[evt.code]) {
                deviceEvents.push(keyEvent(scancode));
            }

            this.doTransactionFromDeviceEvents(deviceEvents);
        }
    }

    updateImageCallback(metadata, buffer) {
        this._updateImage.next({
            pixels: buffer,
            infos: metadata
        });
    }

    connect(username: string, password: string, address: string, authToken: string): Observable<NewSessionInfo> {
        const sessionBuilder = SessionBuilder.new();
        sessionBuilder.address(address);
        sessionBuilder.password(password);
        sessionBuilder.auth_token(authToken);
        sessionBuilder.username(username);
        sessionBuilder.update_callback_context(this);
        sessionBuilder.update_callback(this.updateImageCallback);

        return from(sessionBuilder.connect()).pipe(
            catchError(err => {
                loggingService.error("error:", err);
                userInteractionService.raiseSessionEvent(err);
                return of(err);
            }),
            filter(result => result instanceof Session),
            map((session: Session) => {
                from(session.run()).pipe(
                    catchError(err => {
                        userInteractionService.raiseSessionEvent(err);
                        return of(err);
                    })
                ).subscribe(() => {
                    userInteractionService.raiseSessionEvent("Session was terminated.");
                });
                return session;
            }),
            map((session: Session) => {
                loggingService.info('Session started.')
                this.session = session;
                this._resize.next({
                    desktop_size: session.desktop_size(),
                    session_id: 0
                });
                return {
                    session_id: 0,
                    initial_desktop_size: session.desktop_size(),
                    websocket_port: 0
                }
            }),
        );
    }

    private doTransactionFromDeviceEvents(deviceEvents: DeviceEvent[]) {
        const transaction = InputTransaction.new();
        deviceEvents.forEach(event => transaction.add_event(event));
        this.session?.apply_inputs(transaction);
    }
}
