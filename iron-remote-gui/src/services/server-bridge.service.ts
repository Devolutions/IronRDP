import type {Observable} from "rxjs";
import type {LogType} from '../enums/LogType';

export interface ServerRect {
    free(): void;

    clone_buffer(): Uint8Array;

    bottom: number;
    left: number;
    right: number;
    top: number;
}

export interface NewSessionInfo {
    session_id: number,
    websocket_port: number,
    initial_desktop_size: DesktopSize,
}

export interface DesktopSize {
    width: number,
    height: number
}

export interface ResizeEvent {
    session_id: number,
    desktop_size: DesktopSize,
}

export enum MouseButton {
    LEFT = 0,
    MIDDLE = 1,
    RIGHT = 2,
    BROWSER_BACK = 3,
    BROWSER_FORWARD = 4
}

export enum MouseButtonState {
    MOUSE_DOWN,
    MOUSE_UP
}

export enum SpecialCombination {
    CTRL_ALT_DEL,
    META
}

export abstract class ServerBridgeService {
    abstract init(debug: LogType): void;

    abstract connect(username: string, password: string, address: string, authToken?: string): Observable<NewSessionInfo>;

    abstract resize: Observable<ResizeEvent>;

    abstract updateImage: Observable<any>;

    abstract mouseButtonState(mouse_button: MouseButton, state: MouseButtonState): void;
    
    abstract updateMousePosition(mouse_x: number, mouse_y: number): void;
    
    abstract sendKeyboard(evt: any): void;
    
    abstract releaseAllInputs():void;
    
    abstract sendSpecialCombination(specialCombination: SpecialCombination):void;
    
    abstract syncModifier(evt: any): void;
}

