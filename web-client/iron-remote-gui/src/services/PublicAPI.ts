import {loggingService} from './logging.service';
import type {NewSessionInfo} from '../interfaces/NewSessionInfo';
import {SpecialCombination} from '../enums/SpecialCombination';
import type {WasmBridgeService} from './wasm-bridge.service';
import type {UserInteraction} from '../interfaces/UserInteraction';
import type {ScreenScale} from '../enums/ScreenScale';
import type {Observable} from 'rxjs';

export class PublicAPI {
    private wasmService: WasmBridgeService;

    constructor(wasmService: WasmBridgeService) {
        this.wasmService = wasmService;
    }

    private connect(username: string, password: string, hostname: string, gatewayAddress: string, domain: string, authToken: string): Observable<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        return this.wasmService.connect(username, password, hostname, gatewayAddress, domain, authToken);
    }

    private ctrlAltDel() {
        this.wasmService.sendSpecialCombination(SpecialCombination.CTRL_ALT_DEL);
    }

    private metaKey() {
        this.wasmService.sendSpecialCombination(SpecialCombination.META);
    }

    private setVisibility(state: boolean) {
        loggingService.info(`Change component visibility to: ${state}`);
        this.wasmService.setVisibility(state);
    }

    private setScale(scale: ScreenScale) {
        this.wasmService.setScale(scale);
    }

    getExposedFunctions(): UserInteraction {
        return {
            setVisibility: this.setVisibility.bind(this),
            connect: this.connect.bind(this),
            setScale: this.setScale.bind(this),
            sessionListener: this.wasmService.sessionObserver,
            ctrlAltDel: this.ctrlAltDel.bind(this),
            metaKey: this.metaKey.bind(this)
        }
    }
}

