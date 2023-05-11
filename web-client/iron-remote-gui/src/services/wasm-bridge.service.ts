import {BehaviorSubject, from, Observable, of, Subject} from 'rxjs';
import init, {DeviceEvent, InputTransaction, ironrdp_init, IronRdpError, Session, SessionBuilder} from '../../../../crates/ironrdp-web/pkg/ironrdp_web';
import {loggingService} from './logging.service';
import {catchError, filter, map} from 'rxjs/operators';
import {scanCode} from '../lib/scancodes';
import {LogType} from '../enums/LogType';
import {OS} from '../enums/OS';
import {ModifierKey} from '../enums/ModifierKey';
import {LockKey} from '../enums/LockKey';
import {SessionEventType} from '../enums/SessionEventType';
import type {NewSessionInfo} from '../interfaces/NewSessionInfo';
import {SpecialCombination} from '../enums/SpecialCombination';
import type {ResizeEvent} from '../interfaces/ResizeEvent';
import {ScreenScale} from '../enums/ScreenScale';
import type {MousePosition} from '../interfaces/MousePosition';
import type {SessionEvent} from '../interfaces/session-event';

export class WasmBridgeService {
    private _resize: Subject<ResizeEvent> = new Subject<any>();
    private _updateImage: Subject<any> = new Subject<any>();
    private mousePosition: BehaviorSubject<MousePosition> = new BehaviorSubject<MousePosition>({
        x: 0,
        y: 0
    });
    private changeVisibility: Subject<boolean> = new Subject();
    private sessionEvent: Subject<SessionEvent> = new Subject();
    private scale: BehaviorSubject<ScreenScale> = new BehaviorSubject(ScreenScale.Fit);
    private canvas: HTMLCanvasElement;
    private keyboardActive: boolean;

    resize: Observable<ResizeEvent>;
    updateImage: Observable<any>;
    session?: Session;
    modifierKeyPressed: ModifierKey[] = [];
    mousePositionObservable: Observable<MousePosition> = this.mousePosition.asObservable();
    changeVisibilityObservable: Observable<boolean> = this.changeVisibility.asObservable();
    sessionObserver: Observable<SessionEvent> = this.sessionEvent.asObservable();
    scaleObserver: Observable<ScreenScale> = this.scale.asObservable();

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


    mouseButtonState(event: MouseEvent, isDown: boolean) {
        event.preventDefault(); // prevent default behavior (context menu, etc)
        let mouseFnc = isDown ? DeviceEvent.new_mouse_button_pressed : DeviceEvent.new_mouse_button_released;
        this.doTransactionFromDeviceEvents([mouseFnc(event.button)]);
    }

    updateMousePosition(position: MousePosition) {
        if (!this.keyboardActive) {
            this.keyboardActive = true;
        }
        this.doTransactionFromDeviceEvents([DeviceEvent.new_mouse_move(position.x, position.y)]);
        this.mousePosition.next(position);
    }


    connect(username: string, password: string, destination: string, proxyAddress: string, serverDomain: string, authToken: string): Observable<NewSessionInfo> {
        const sessionBuilder = SessionBuilder.new();
        sessionBuilder.proxy_address(proxyAddress);
        sessionBuilder.destination(destination);
        sessionBuilder.server_domain(serverDomain);
        sessionBuilder.password(password);
        sessionBuilder.auth_token(authToken);
        sessionBuilder.username(username);
        sessionBuilder.update_callback_context(this);
        sessionBuilder.update_callback(this.updateImageCallback);

        return from(sessionBuilder.connect()).pipe(
            catchError((err: IronRdpError) => {
                this.raiseSessionEvent({
                    type: SessionEventType.ERROR,
                    data: err
                });
                return of(err);
            }),
            filter(result => result instanceof Session),
            map((session: Session) => {
                from(session.run()).pipe(
                    catchError(err => {
                        this.setVisibility(false);
                        this.raiseSessionEvent({
                            type: SessionEventType.ERROR,
                            data: err.backtrace()
                        });
                        this.raiseSessionEvent({
                            type: SessionEventType.TERMINATED,
                            data: 'Session was terminated.'
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
                this.raiseSessionEvent({
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


    mouseWheel(event) {
        let vertical = event.deltaY !== 0;
        let rotation = vertical ? event.deltaY : event.deltaX;
        this.doTransactionFromDeviceEvents([DeviceEvent.new_wheel_rotations(vertical, rotation)]);
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

    private releaseAllInputs() {
        this.session?.release_all_inputs();
    }

    private sendKeyboard(evt: KeyboardEvent) {
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

    private updateImageCallback(metadata, buffer) {
        this._updateImage.next({
            pixels: buffer,
            infos: metadata
        });
    }

    private syncModifier(evt: any): void {
        const mouseEvent = evt as MouseEvent;

        let syncCapsLockActive = mouseEvent.getModifierState(LockKey.CAPS_LOCK);
        let syncNumsLockActive = mouseEvent.getModifierState(LockKey.NUM_LOCK);
        let syncScrollLockActive = mouseEvent.getModifierState(LockKey.SCROLL_LOCK);
        let syncKanaModeActive = mouseEvent.getModifierState(LockKey.KANA_MODE);

        this.session.synchronize_lock_keys(syncScrollLockActive, syncNumsLockActive, syncCapsLockActive, syncKanaModeActive);
    }

    private raiseSessionEvent(event: SessionEvent) {
        this.sessionEvent.next(event);
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
        const ctrl = scanCode('ControlLeft', OS.WINDOWS);
        const alt = scanCode('AltLeft', OS.WINDOWS);
        const suppr = scanCode('Delete', OS.WINDOWS);

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
        const ctrl = scanCode('ControlLeft', OS.WINDOWS);
        const escape = scanCode('Escape', OS.WINDOWS);

        this.doTransactionFromDeviceEvents([
            DeviceEvent.new_key_pressed(ctrl),
            DeviceEvent.new_key_pressed(escape),
            DeviceEvent.new_key_released(ctrl),
            DeviceEvent.new_key_released(escape),
        ]);
    }
}
