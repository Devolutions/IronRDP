import type {NewSessionInfo, ResizeEvent, ServerBridgeService} from './server-bridge.service';
import {MouseButton, MouseButtonState} from './server-bridge.service';
import {from, Observable, of, Subject} from 'rxjs';
import init, {DeviceEvent, InputTransaction, ironrdp_init, Session, SessionBuilder} from "../../../ffi/wasm/pkg/ironrdp";
import {loggingService} from "./logging.service";
import {catchError, filter, map} from "rxjs/operators";
import {userInteractionService} from "./user-interaction-service";

const modifierKey = {
    SHIFT: 16,
    CTRL: 17,
    ALT: 18,
    META: 91
};
const modifierKeyArray = [modifierKey.ALT, modifierKey.CTRL, modifierKey.ALT, modifierKey.META];

export class WasmBridgeService implements ServerBridgeService {
    private _resize: Subject<ResizeEvent> = new Subject<any>();
    private _updateImage: Subject<any> = new Subject<any>();

    resize: Observable<ResizeEvent>;
    updateImage: Observable<any>;

    session?: Session;

    constructor() {
        this.resize = this._resize.asObservable();
        this.updateImage = this._updateImage.asObservable();
        loggingService.info('Web bridge initialized.');
    }

    async init(debug: "OFF" | "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE") {
        loggingService.info('Loading wasm file.');
        await init();
        loggingService.info('Initializing IronRDP.');
        ironrdp_init(debug);
    }

    mouseButtonState(mouse_button: MouseButton, state: MouseButtonState) {
        const transaction = InputTransaction.new();
        let mouseFnc = state === MouseButtonState.MOUSE_DOWN ? DeviceEvent.new_mouse_button_pressed : DeviceEvent.new_mouse_button_released;
        transaction.add_event(mouseFnc(mouse_button));
        this.session?.apply_inputs(transaction);
    }
    
    updateMousePosition(mouse_x: number, mouse_y: number) {
        const transaction = InputTransaction.new();
        transaction.add_event(DeviceEvent.new_mouse_move(mouse_x, mouse_y));
        this.session?.apply_inputs(transaction);
    }

    sendKeyboard(evt: KeyboardEvent) {
        let keyEvent;
        
        if (evt.type === 'keypress') {
            keyEvent = DeviceEvent.new_key_pressed;
        } else if (evt.type === 'keyup') {
            keyEvent = DeviceEvent.new_key_released;
        }

        if (keyEvent) {
            const transaction = InputTransaction.new();

            // NOTE: There is no keypress event for alt, ctrl, shift and meta keys, so we check manually.
            // TODO: Support for right side
            // TODO: Support for meta key (also called os key)
            if (evt.altKey && evt.code !== "AltLeft") {
                transaction.add_event(DeviceEvent.new_key_pressed(0x38));
            }
            if (evt.ctrlKey && evt.code !== "ControlLeft") {
                transaction.add_event(DeviceEvent.new_key_pressed(0x1D));
            }
            if (evt.shiftKey && evt.code !== "ShiftLeft") {
                transaction.add_event(DeviceEvent.new_key_pressed(0x2A));
            }

            // NOTE: We only receive a keyup event for Backspace
            if (evt.code === "Backspace") {
                transaction.add_event(DeviceEvent.new_key_pressed(0x0E));
            }

            const scancode = this.convertToScancode(evt.code);
            transaction.add_event(keyEvent(scancode));
            
            this.session?.apply_inputs(transaction);
        }
    }

    // Temporary workaround for scancode
    convertToScancode(code: string): number {
        // From: https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_code_values
        switch (code) {
            case "Escape":
                return 0x01;
            case "Digit1":
                return 0x02;
            case "Digit2":
                return 0x03;
            case "Digit3":
                return 0x04;
            case "Digit4":
                return 0x05;
            case "Digit5":
                return 0x06;
            case "Digit6":
                return 0x07;
            case "Digit7":
                return 0x08;
            case "Digit8":
                return 0x09;
            case "Digit9":
                return 0x0A;
            case "Digit0":
                return 0x0B;
            case "Minus":
                return 0x0C;
            case "Equal":
                return 0x0D;
            case "Backspace":
                return 0x0E;
            case "Tab":
                return 0x0F;
            case "KeyQ":
                return 0x10;
            case "KeyW":
                return 0x11;
            case "KeyE":
                return 0x12;
            case "KeyR":
                return 0x13;
            case "KeyT":
                return 0x14;
            case "KeyY":
                return 0x15;
            case "KeyU":
                return 0x16;
            case "KeyI":
                return 0x17;
            case "KeyO":
                return 0x18;
            case "KeyP":
                return 0x19;
            case "BracketLeft":
                return 0x1A;
            case "BracketRight":
                return 0x1B;
            case "Enter":
                return 0x1C;
            case "ControlLeft":
                return 0x1D;
            case "KeyA":
                return 0x1E;
            case "KeyS":
                return 0x1F;
            case "KeyD":
                return 0x20;
            case "KeyF":
                return 0x21;
            case "KeyG":
                return 0x22;
            case "KeyH":
                return 0x23;
            case "KeyJ":
                return 0x24;
            case "KeyK":
                return 0x25;
            case "KeyL":
                return 0x26;
            case "Semicolon":
                return 0x27;
            case "Quote":
                return 0x28;
            case "Backquote":
                return 0x29;
            case "ShiftLeft":
                return 0x2A;
            case "Backslash":
                return 0x2B;
            case "KeyZ":
                return 0x2C;
            case "KeyX":
                return 0x2D;
            case "KeyC":
                return 0x2E;
            case "KeyV":
                return 0x2F;
            case "KeyB":
                return 0x30;
            case "KeyN":
                return 0x31;
            case "KeyM":
                return 0x32;
            case "Comma":
                return 0x33;
            case "Period":
                return 0x34;
            case "Slash":
                return 0x35;
            case "ShiftRight":
                return 0x36;
            case "NumpadMultiply":
                return 0x37;
            case "AltLeft":
                return 0x38;
            case "Space":
                return 0x39;
            case "CapsLock":
                return 0x3A;
            case "F1":
                return 0x3B;
            case "F2":
                return 0x3C;
            case "F3":
                return 0x3D;
            case "F4":
                return 0x3E;
            case "F5":
                return 0x3F;
            case "F6":
                return 0x40;
            case "F7":
                return 0x41;
            case "F8":
                return 0x42;
            case "F9":
                return 0x43;
            case "F10":
                return 0x44;
            case "Pause":
                return 0x45;
            case "ScrollLock":
                return 0x46;
            default:
                return 0x00;
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
}
