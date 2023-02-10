import type {NewSessionInfo, ResizeEvent, ServerBridgeService} from './server-bridge.service';
import {MouseButton, MouseButtonState, SpecialCombination} from './server-bridge.service';
import {from, Observable, of, Subject} from 'rxjs';
import init, {DeviceEvent, InputTransaction, ironrdp_init, Session, SessionBuilder} from "../../../ffi/wasm/pkg/ironrdp";
import {loggingService} from "./logging.service";
import {catchError, filter, map} from "rxjs/operators";
import {userInteractionService} from "./user-interaction-service";
import {scanCode} from '../lib/scancodes';
import {LogType} from '../enums/LogType';
import {OS} from '../enums/OS';
import {ModifierKey} from '../enums/ModifierKey';
import {LockKey} from '../enums/LockKey';

export class WasmBridgeService implements ServerBridgeService {
    private _resize: Subject<ResizeEvent> = new Subject<any>();
    private _updateImage: Subject<any> = new Subject<any>();

    resize: Observable<ResizeEvent>;
    updateImage: Observable<any>;
    session?: Session;

    modifierKeyPressed: ModifierKey[] = [];
    lockKeyPressed: LockKey[] = [];

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
                this.updateModifierKeyState(evt);
            }

            if (LockKey[evt.code]) {
                this.updateLockKeyState(evt);
            }

            deviceEvents.push(keyEvent(scancode));

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

    syncModifier(evt: any): void {
        const mouseEvent = evt as MouseEvent;
        const events = [];

        let syncCapsLock_On = mouseEvent.getModifierState(LockKey.CAPS_LOCK) && this.lockKeyPressed.indexOf(LockKey.CAPS_LOCK) === -1;
        let syncCapsLock_Off = !mouseEvent.getModifierState(LockKey.CAPS_LOCK) && this.lockKeyPressed.indexOf(LockKey.CAPS_LOCK) > -1;
        let syncNumsLock_On = mouseEvent.getModifierState(LockKey.NUMS_LOCK) && this.lockKeyPressed.indexOf(LockKey.NUMS_LOCK) === -1;
        let syncNumsLock_Off = !mouseEvent.getModifierState(LockKey.NUMS_LOCK) && this.lockKeyPressed.indexOf(LockKey.NUMS_LOCK) > -1;
        let syncScrollLock_On = mouseEvent.getModifierState(LockKey.SCROLL_LOCK) && this.lockKeyPressed.indexOf(LockKey.SCROLL_LOCK) === -1;
        let syncScrollLock_Off = !mouseEvent.getModifierState(LockKey.SCROLL_LOCK) && this.lockKeyPressed.indexOf(LockKey.SCROLL_LOCK) > -1;

        if (syncCapsLock_On || syncCapsLock_Off) {
            events.push(DeviceEvent.new_key_pressed(scanCode(LockKey.CAPS_LOCK, OS.WINDOWS)));
            events.push(DeviceEvent.new_key_released(scanCode(LockKey.CAPS_LOCK, OS.WINDOWS)));
            if (syncCapsLock_On) {
                this.lockKeyPressed.push(LockKey.CAPS_LOCK);
            } else {
                this.lockKeyPressed.splice(this.lockKeyPressed.indexOf(LockKey.CAPS_LOCK), 1);
            }
        }
        if (syncNumsLock_On || syncNumsLock_Off) {
            events.push(DeviceEvent.new_key_pressed(scanCode(LockKey.NUMS_LOCK, OS.WINDOWS)));
            events.push(DeviceEvent.new_key_released(scanCode(LockKey.NUMS_LOCK, OS.WINDOWS)));
            if (syncNumsLock_On) {
                this.lockKeyPressed.push(LockKey.NUMS_LOCK);
            } else {
                this.lockKeyPressed.splice(this.lockKeyPressed.indexOf(LockKey.NUMS_LOCK), 1);
            }
        }
        if (syncScrollLock_On || syncScrollLock_Off) {
            events.push(DeviceEvent.new_key_pressed(scanCode(LockKey.SCROLL_LOCK, OS.WINDOWS)));
            events.push(DeviceEvent.new_key_released(scanCode(LockKey.SCROLL_LOCK, OS.WINDOWS)));
            if (syncScrollLock_On) {
                this.lockKeyPressed.push(LockKey.SCROLL_LOCK);
            } else {
                this.lockKeyPressed.splice(this.lockKeyPressed.indexOf(LockKey.SCROLL_LOCK), 1);
            }
        }

        this.doTransactionFromDeviceEvents(events);
    }
    
    private updateModifierKeyState(evt) {
        if (this.modifierKeyPressed.indexOf(ModifierKey[evt.code]) === -1) {
            this.modifierKeyPressed.push(ModifierKey[evt.code]);
        } else if (evt.type === 'keyup') {
            this.modifierKeyPressed.splice(this.modifierKeyPressed.indexOf(ModifierKey[evt.code]), 1);
        }
    }

    private updateLockKeyState(evt) {
        if (this.lockKeyPressed.indexOf(LockKey[evt.code]) === -1) {
            this.lockKeyPressed.push(LockKey[evt.code]);
        } else if (evt.type === 'keyup' && !evt.getModifierState(evt.code)) {
            this.lockKeyPressed.splice(this.lockKeyPressed.indexOf(LockKey[evt.code]), 1);
        }
    }
    
    private doTransactionFromDeviceEvents(deviceEvents: DeviceEvent[]) {
        const transaction = InputTransaction.new();
        deviceEvents.forEach(event => transaction.add_event(event));
        this.session?.apply_inputs(transaction);
    }

    private ctrlAltDel() {
        const ctrl = scanCode("ControlLeft", OS.WINDOWS);
        const alt = scanCode("AltLeft", OS.WINDOWS);
        const suppr = scanCode("Delete", OS.WINDOWS);

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
        const ctrl = scanCode("ControlLeft", OS.WINDOWS);
        const escape = scanCode("Escape", OS.WINDOWS);

        this.doTransactionFromDeviceEvents([
            DeviceEvent.new_key_pressed(ctrl),
            DeviceEvent.new_key_pressed(escape),
            DeviceEvent.new_key_released(ctrl),
            DeviceEvent.new_key_released(escape),
        ]);
    }
}
