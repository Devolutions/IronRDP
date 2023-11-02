import type { NewSessionInfo, ResizeEvent, ServerBridgeService } from './server-bridge.service';
import * as IronWasm from '../../../ffi/wasm/pkg/ironrdp_web';
import { Observable, of, Subject } from 'rxjs';

export class WasmBridgeService implements ServerBridgeService {
	private wasmBridge = IronWasm;

	private _resize: Subject<ResizeEvent> = new Subject<ResizeEvent>();

	resize: Observable<ResizeEvent>;

	constructor() {
		this.resize = this._resize.asObservable();
	}

	init(): void {
		this.wasmBridge.init();
	}

	updateMouse(_mouse_x: number, _mouse_y: number, _click_state: number): void {
		// Not implemented yet...
	}

	connect(username: string, password: string, address: string): Observable<NewSessionInfo> {
		this.wasmBridge.connect(username, password, address);
		return of({
			session_id: 0,
			initial_desktop_size: {
				height: 0,
				width: 0
			},
			websocket_port: 0
		});
	}
}
