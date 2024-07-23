import { loggingService } from './logging.service';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import type { WasmBridgeService } from './wasm-bridge.service';
import type { UserInteraction } from '../interfaces/UserInteraction';
import type { ScreenScale } from '../enums/ScreenScale';
import type { DesktopSize } from '../interfaces/DesktopSize';

export class PublicAPI {
    private wasmService: WasmBridgeService;

    constructor(wasmService: WasmBridgeService) {
        this.wasmService = wasmService;
    }

    private connect(
        username: string,
        password: string,
        destination: string,
        proxyAddress: string,
        serverDomain: string,
        authToken: string,
        desktopSize?: DesktopSize,
        preConnectionBlob?: string,
        kdc_proxy_url?: string,
    ): Promise<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        const resultObservable = this.wasmService.connect(
            username,
            password,
            destination,
            proxyAddress,
            serverDomain,
            authToken,
            desktopSize,
            preConnectionBlob,
            kdc_proxy_url,
        );

        return resultObservable.toPromise();
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

    private shutdown() {
        this.wasmService.shutdown();
    }

    private setKeyboardUnicodeMode(use_unicode: boolean) {
        this.wasmService.setKeyboardUnicodeMode(use_unicode);
    }

    private setCursorStyleOverride(style: string | null) {
        this.wasmService.setCursorStyleOverride(style);
    }

    getExposedFunctions(): UserInteraction {
        return {
            setVisibility: this.setVisibility.bind(this),
            connect: this.connect.bind(this),
            setScale: this.setScale.bind(this),
            onSessionEvent: (callback) => {
                this.wasmService.sessionObserver.subscribe(callback);
            },
            ctrlAltDel: this.ctrlAltDel.bind(this),
            metaKey: this.metaKey.bind(this),
            shutdown: this.shutdown.bind(this),
            setKeyboardUnicodeMode: this.setKeyboardUnicodeMode.bind(this),
            setCursorStyleOverride: this.setCursorStyleOverride.bind(this),
        };
    }
}
