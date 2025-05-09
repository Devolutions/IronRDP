import { loggingService } from './logging.service';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import { RemoteDesktopService } from './remote-desktop.service';
import type { UserInteraction } from '../interfaces/UserInteraction';
import type { ScreenScale } from '../enums/ScreenScale';
import { ConfigBuilder } from './ConfigBuilder';
import { Config } from './Config';

export class PublicAPI {
    private remoteDesktopService: RemoteDesktopService;

    constructor(remoteDesktopService: RemoteDesktopService) {
        this.remoteDesktopService = remoteDesktopService;
    }

    private configBuilder(): ConfigBuilder {
        return this.remoteDesktopService.configBuilder();
    }

    private connect(config: Config): Promise<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        const resultObservable = this.remoteDesktopService.connect(config);

        return resultObservable.toPromise();
    }

    private ctrlAltDel() {
        this.remoteDesktopService.sendSpecialCombination(SpecialCombination.CTRL_ALT_DEL);
    }

    private metaKey() {
        this.remoteDesktopService.sendSpecialCombination(SpecialCombination.META);
    }

    private setVisibility(state: boolean) {
        loggingService.info(`Change component visibility to: ${state}`);
        this.remoteDesktopService.setVisibility(state);
    }

    private setScale(scale: ScreenScale) {
        this.remoteDesktopService.setScale(scale);
    }

    private shutdown() {
        this.remoteDesktopService.shutdown();
    }

    private setKeyboardUnicodeMode(use_unicode: boolean) {
        this.remoteDesktopService.setKeyboardUnicodeMode(use_unicode);
    }

    private setCursorStyleOverride(style: string | null) {
        this.remoteDesktopService.setCursorStyleOverride(style);
    }

    private resize(width: number, height: number, scale?: number) {
        this.remoteDesktopService.resizeDynamic(width, height, scale);
    }

    private setEnableClipboard(enable: boolean) {
        this.remoteDesktopService.setEnableClipboard(enable);
    }

    getExposedFunctions(): UserInteraction {
        return {
            setVisibility: this.setVisibility.bind(this),
            configBuilder: this.configBuilder.bind(this),
            connect: this.connect.bind(this),
            setScale: this.setScale.bind(this),
            onSessionEvent: (partialObserver) => {
                this.remoteDesktopService.sessionObserver.subscribe(partialObserver);
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
