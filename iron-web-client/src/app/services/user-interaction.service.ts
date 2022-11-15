import {Injectable} from "@angular/core";
import {BehaviorSubject, Observable, Subject} from "rxjs";
import {ServerBridgeService} from "./server-bridge.service";

export interface MousePosition {
  x: number,
  y: number
}

@Injectable()
export class UserInteractionService {

  mouseLeftClickObservable: Observable<number>;
  private mouseLeftClick: BehaviorSubject<number> = new BehaviorSubject<number>(0);
  mousePositionObservable: Observable<MousePosition>;
  private mousePosition: BehaviorSubject<MousePosition> = new BehaviorSubject<MousePosition>({x: 0, y: 0});

  constructor(private serverBridge: ServerBridgeService) {
    this.mousePositionObservable = this.mousePosition.asObservable();
    this.mouseLeftClickObservable = this.mouseLeftClick.asObservable();
  }

  setMousePosition(position: MousePosition) {
    this.serverBridge.updateMouse(position.x, position.y, this.mouseLeftClick.value);
    this.mousePosition.next(position);
  }

  setMouseLeftClickState(state: number) {
    this.serverBridge.updateMouse(this.mousePosition.value.x, this.mousePosition.value.y, state);
    this.mouseLeftClick.next(state);
  }
}
