import type { NewSessionInfo, ServerBridgeService } from './server-bridge.service';
import * as IronWasm from '../../../ffi/wasm/pkg/ironrdp_web';
import { Observable, of, Subject } from 'rxjs';

export class WasmBridgeService implements ServerBridgeService {
	private wasmBridge = IronWasm;

	private _resize: Subject<any> = new Subject<any>();

	resize: Observable<any>;

	constructor() {
		this.resize = this._resize.asObservable();
	}

	init(): void {
		this.wasmBridge.init();
	}

	updateMouse(mouse_x: number, mouse_y: number, click_state: number): void {
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
