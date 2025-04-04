import { loggingService } from './logging.service';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import type { RemoteDesktopModule } from '../interfaces/RemoteDesktopModule';
import { WasmBridgeService } from './wasm-bridge.service';
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
        use_display_control = false,
    ): Promise<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        if (this.wasmService === undefined) {
            return Promise.reject(new Error('backend was never set'));
        }
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
            use_display_control,
        );

        return resultObservable.toPromise();
    }

    private ctrlAltDel() {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.sendSpecialCombination(SpecialCombination.CTRL_ALT_DEL);
    }

    private metaKey() {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.sendSpecialCombination(SpecialCombination.META);
    }

    private setVisibility(state: boolean) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        loggingService.info(`Change component visibility to: ${state}`);
        this.wasmService.setVisibility(state);
    }

    private setScale(scale: ScreenScale) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.setScale(scale);
    }

    private shutdown() {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.shutdown();
    }

    private setKeyboardUnicodeMode(use_unicode: boolean) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.setKeyboardUnicodeMode(use_unicode);
    }

    private setCursorStyleOverride(style: string | null) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.setCursorStyleOverride(style);
    }

    private resize(width: number, height: number, scale?: number) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.resizeDynamic(width, height, scale);
    }

    private setEnableClipboard(enable: boolean) {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
        this.wasmService.setEnableClipboard(enable);
    }

    getExposedFunctions(): UserInteraction {
        if (this.wasmService === undefined) {
            throw new Error('backend was never set');
        }
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
            resize: this.resize.bind(this),
            setEnableClipboard: this.setEnableClipboard.bind(this),
        };
    }
}
