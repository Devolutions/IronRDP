import type {NewSessionInfo, ServerBridgeService} from "./server-bridge.service";
import {invoke} from "@tauri-apps/api";
import {from, Observable, Subject} from "rxjs";
import {listen} from "@tauri-apps/api/event";
import {loggingService} from "./logging.service";
import {tap} from "rxjs/operators";
import type {MouseButton, MouseButtonState} from './server-bridge.service';

export class TauriBridgeService implements ServerBridgeService {

    private _resize: Subject<any> = new Subject<any>();
    private _updateImage: Subject<any> = new Subject<any>();

    private lastImageInformations = '';

    resize: Observable<any>;
    updateImage: Observable<any>;

    constructor() {
        this.resize = this._resize.asObservable();
        this.updateImage = this._updateImage.asObservable();

        this.initTauriListener();
        loggingService.info('Native bridge initialized.');
    }

    init(): void {
        //Nothing to do
    }

    connect(username: string, password: string, address: string): Observable<NewSessionInfo> {
        return from(invoke("connect", {username, password, address}) as Promise<NewSessionInfo>).pipe(
            tap((newSessionInfo: NewSessionInfo) => {
                this.initSocket(newSessionInfo.websocket_port);
            }));
    }

    initSocket(port: any) {
        const socket = new WebSocket(`ws://127.0.0.1:${port}`);
        socket.addEventListener("message", this.onSocketMessage.bind(this));
    }

    updateMouse(mouseX: number, mouseY: number, clickState: number) {
        const leftClick = clickState === 0;
        invoke("update_mouse", {sessionId: 0, mouseX, mouseY, leftClick});
    }

    async onSocketMessage(event: any) {
        if (typeof event.data === "string") {
            this.lastImageInformations = event.data;
        } else {
            const obj = {
                pixels: event.data.arrayBuffer(),
                infos: JSON.parse(this.lastImageInformations)
            }
            this._updateImage.next(obj);
        }
    }

    private async initTauriListener() {
        const unlisten1 = await listen("resize", (evt: any) => {
            this._resize.next(evt.payload);
        })
    }

    sendKeyboard(evt: any): void {
    }

    mouseButtonState(mouse_button: MouseButton, state: MouseButtonState): void {
    }

    updateMousePosition(mouse_x: number, mouse_y: number): void {
    }
}
