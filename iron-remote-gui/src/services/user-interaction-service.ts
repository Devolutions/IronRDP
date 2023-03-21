import {BehaviorSubject, Observable, Subject} from 'rxjs';
import type {NewSessionInfo, ServerBridgeService} from "./server-bridge.service";
import {MouseButtonState, SpecialCombination} from './server-bridge.service';
import {loggingService} from "./logging.service";
import type {SessionEvent} from '../interfaces/session-event.model';

export interface MousePosition {
    x: number;
    y: number;
}

export enum ScreenScale {
    Fit = 1,
    Full = 2,
    Real = 3
}

export interface IRGUserInteraction {
    setMousePosition(position: MousePosition);

    setMouseButtonState(event: MouseEvent, isDown: boolean);

    setVisibility(state: boolean);

    setScale(scale: ScreenScale);

    connect(username: string, password: string, hostname: string, gatewayAddress: string, domain: string, authToken: string): Observable<NewSessionInfo>;

    ctrlAltDel();

    metaKey();

    sessionListener: Observable<any>;
}

export class UserInteractionService {
    private mousePosition: BehaviorSubject<MousePosition> = new BehaviorSubject<MousePosition>({
        x: 0,
        y: 0
    });
    mousePositionObservable: Observable<MousePosition> = this.mousePosition.asObservable();

    private changeVisibility: Subject<boolean> = new Subject();
    changeVisibilityObservable: Observable<boolean> = this.changeVisibility.asObservable();

    private sessionEvent: Subject<SessionEvent> = new Subject();
    sessionObserver: Observable<SessionEvent> = this.sessionEvent.asObservable();

    private serverBridge: ServerBridgeService;

    private scale: BehaviorSubject<ScreenScale> = new BehaviorSubject(ScreenScale.Fit);
    scaleObserver: Observable<ScreenScale> = this.scale.asObservable();

    private canvas: HTMLCanvasElement;

    private keyboardActive: boolean;

    setServerBridge(serverBridge: ServerBridgeService) {
        this.serverBridge = serverBridge;
    }

    setCanvas(canvas: HTMLCanvasElement) {
        this.canvas = canvas;
    }

    connect(username: string, password: string, hostname: string, gatewayAddress: string, domain: string, authToken: string): Observable<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        return this.serverBridge.connect(username, password, hostname, gatewayAddress, domain, authToken);
    }

    mouseIn(event: MouseEvent) {
        this.serverBridge.syncModifier(event);
        this.keyboardActive = true;
    }

    mouseOut(event: MouseEvent) {
        this.keyboardActive = false;
        this.serverBridge?.releaseAllInputs();
    }

    setMousePosition(position: MousePosition) {
        if (!this.keyboardActive) {
            this.keyboardActive = true;
        }
        this.serverBridge?.updateMousePosition(position.x, position.y);
        this.mousePosition.next(position);
    };

    setMouseButtonState(event: MouseEvent, isDown: boolean) {
        event.preventDefault(); // prevent default behavior (context menu, etc)
        this.serverBridge?.mouseButtonState(event.button, isDown ? MouseButtonState.MOUSE_DOWN : MouseButtonState.MOUSE_UP);
    }

    sendKeyboardEvent(evt: KeyboardEvent) {
        if (this.keyboardActive) {
            this.serverBridge.sendKeyboard(evt);
        }
    }

    ctrlAltDel() {
        this.sendSpecialCombination(SpecialCombination.CTRL_ALT_DEL);
    }

    metaKey() {
        this.sendSpecialCombination(SpecialCombination.META);
    }

    private sendSpecialCombination(combination: SpecialCombination) {
        this.serverBridge.sendSpecialCombination(combination);
    }

    setVisibility(state: boolean) {
        loggingService.info(`Change component visibility to: ${state}`);
        this.changeVisibility.next(state);
    }

    setScale(scale: ScreenScale) {
        this.scale.next(scale);
    }

    raiseSessionEvent(event: SessionEvent) {
        this.sessionEvent.next(event);
    }

    mouseWheel(event) {
        let vertical = event.deltaY !== 0;
        let rotations = vertical ? event.deltaY : event.deltaX;
        this.serverBridge.mouseWheel(vertical, -rotations);
    }

    exposedFunctions: IRGUserInteraction = {
        setMousePosition: this.setMousePosition.bind(this),
        setMouseButtonState: this.setMouseButtonState.bind(this),
        setVisibility: this.setVisibility.bind(this),
        connect: this.connect.bind(this),
        setScale: this.setScale.bind(this),
        sessionListener: this.sessionObserver,
        ctrlAltDel: this.ctrlAltDel.bind(this),
        metaKey: this.metaKey.bind(this)
    }
}

export let userInteractionService: UserInteractionService = new UserInteractionService();
