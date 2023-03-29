import type {NewSessionInfo, ResizeEvent, ServerBridgeService} from './server-bridge.service';
import {MouseButton, MouseButtonState, SpecialCombination} from './server-bridge.service';
import {from, Observable, of, Subject} from 'rxjs';
import init, {DeviceEvent, InputTransaction, ironrdp_init, IronRdpError, IronRdpErrorKind, Session, SessionBuilder} from "../../../../crates/web/pkg/ironrdp_web";
import {loggingService} from "./logging.service";
import {catchError, filter, finalize, map} from "rxjs/operators";
import {userInteractionService} from "./user-interaction-service";
import {scanCode} from '../lib/scancodes';
import {LogType} from '../enums/LogType';
import {OS} from '../enums/OS';
import {ModifierKey} from '../enums/ModifierKey';
import {LockKey} from '../enums/LockKey';
import {SessionEventType} from '../enums/SessionEventType';

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
            if (ModifierKey[evt.code]) {
                this.updateModifierKeyState(evt);
            }

            if (LockKey[evt.code]) {
                this.syncModifier(evt);
            }

            if (!evt.repeat || (!ModifierKey[evt.code] && !LockKey[evt.code])) {
                this.doTransactionFromDeviceEvents([keyEvent(scanCode(evt.code, OS.WINDOWS))]);
            }
        }
    }

    updateImageCallback(metadata, buffer) {
        this._updateImage.next({
            pixels: buffer,
            infos: metadata
        });
    }

    connect(username: string, password: string, hostname: string, gatewayAddress: string, domain: string, authToken: string): Observable<NewSessionInfo> {
        const sessionBuilder = SessionBuilder.new();
        sessionBuilder.gateway_address(gatewayAddress);
        sessionBuilder.hostname(hostname);
        sessionBuilder.domain(domain);
        sessionBuilder.password(password);
        sessionBuilder.auth_token(authToken);
        sessionBuilder.username(username);
        sessionBuilder.update_callback_context(this);
        sessionBuilder.update_callback(this.updateImageCallback);

        return from(sessionBuilder.connect()).pipe(
            catchError((err: IronRdpError) => {
                userInteractionService.raiseSessionEvent({
                    type: SessionEventType.ERROR,
                    data: err
                });
                return of(err);
            }),
            filter(result => result instanceof Session),
            map((session: Session) => {
                from(session.run()).pipe(
                    catchError(err => {
                        userInteractionService.setVisibility(false);
                        userInteractionService.raiseSessionEvent({
                            type: SessionEventType.ERROR,
                            data: err.backtrace()
                        });
                        userInteractionService.raiseSessionEvent({
                            type: SessionEventType.TERMINATED,
                            data: "Session was terminated."
                        });
                        return of(err);
                    }),
                ).subscribe();
                return session;
            }),
            map((session: Session) => {
                loggingService.info('Session started.');
                this.session = session;
                this._resize.next({
                    desktop_size: session.desktop_size(),
                    session_id: 0
                });
                userInteractionService.raiseSessionEvent({
                    type: SessionEventType.STARTED,
                    data: 'Session started'
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

        let syncCapsLockActive = mouseEvent.getModifierState(LockKey.CAPS_LOCK);
        let syncNumsLockActive = mouseEvent.getModifierState(LockKey.NUM_LOCK);
        let syncScrollLockActive = mouseEvent.getModifierState(LockKey.SCROLL_LOCK);
        let syncKanaModeActive = mouseEvent.getModifierState(LockKey.KANA_MODE);

        this.session.synchronize_lock_keys(syncScrollLockActive, syncNumsLockActive, syncCapsLockActive, syncKanaModeActive);
    }

    mouseWheel(vertical: boolean, rotation: number) {
        this.doTransactionFromDeviceEvents([DeviceEvent.new_wheel_rotations(vertical, rotation)]);
    }

    private updateModifierKeyState(evt) {
        if (this.modifierKeyPressed.indexOf(ModifierKey[evt.code]) === -1) {
            this.modifierKeyPressed.push(ModifierKey[evt.code]);
        } else if (evt.type === 'keyup') {
            this.modifierKeyPressed.splice(this.modifierKeyPressed.indexOf(ModifierKey[evt.code]), 1);
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
