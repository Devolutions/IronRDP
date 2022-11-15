import {Injectable} from "@angular/core";
import {NewSessionInfo, ServerBridgeService, ServerRect} from "./server-bridge.service";
import {invoke} from "@tauri-apps/api";
import {from, Observable, Subject, tap} from "rxjs";
import {listen} from "@tauri-apps/api/event";

@Injectable()
export class TauriBridgeService implements ServerBridgeService {

  private _resize: Subject<any> = new Subject<any>();
  private _updateImage: Subject<any> = new Subject<any>();

  private lastImageInformations: string;

  resize: Observable<any>;
  updateImage: Observable<any>;

  constructor() {
    this.resize = this._resize.asObservable();
    this.updateImage = this._updateImage.asObservable();

    this.initTauriListener();
  }

  init(): void {
  }

  connect(username: string, password: string, address: string): Observable<any> {
    return from(invoke("connect", {username, password, address}) as Promise<any>).pipe(tap((newSessionInfo: NewSessionInfo) => {
      this.initSocket(newSessionInfo.websocket_port);
    }));
  }

  initSocket(port: any) {
    const socket = new WebSocket(`ws://127.0.0.1:${port}`);
    socket.addEventListener("message", this.onSocketMessage.bind(this));
  }

  updateMouse(mouseX: number, mouseY:number, clickState:number) {
    let leftClick = clickState === 0 ? false : true;
    invoke("update_mouse", {sessionId: 0, mouseX, mouseY, leftClick});
  }

  async onSocketMessage(event: any) {
      if (typeof event.data === "string") {
        this.lastImageInformations = event.data;
      } else {
        let obj = {
          pixels: event.data.arrayBuffer(),
          infos: JSON.parse(this.lastImageInformations)
        }
        this._updateImage.next(obj);
      }
  }

  private async initTauriListener() {
    let unlisten1 = await listen("resize", (evt: any) => {
      this._resize.next(evt.payload);
    })
  }


}
