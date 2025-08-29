import { loggingService } from './logging.service';
import type { NewSessionInfo } from '../interfaces/NewSessionInfo';
import { SpecialCombination } from '../enums/SpecialCombination';
import { RemoteDesktopService } from './remote-desktop.service';
import type { UserInteraction } from '../interfaces/UserInteraction';
import type { ScreenScale } from '../enums/ScreenScale';
import { ConfigBuilder } from './ConfigBuilder';
import { Config } from './Config';
import type { Extension } from '../interfaces/Extension';
import type { ClipboardService } from './clipboard.service';

export class PublicAPI {
    private remoteDesktopService: RemoteDesktopService;
    private clipboardService: ClipboardService;

    constructor(remoteDesktopService: RemoteDesktopService, clipboardService: ClipboardService) {
        this.remoteDesktopService = remoteDesktopService;
        this.clipboardService = clipboardService;
    }

    private configBuilder(): ConfigBuilder {
        return this.remoteDesktopService.configBuilder();
    }

    private connect(config: Config): Promise<NewSessionInfo> {
        loggingService.info('Initializing connection.');
        return this.remoteDesktopService.connect(config);
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

    private setEnableAutoClipboard(enable: boolean) {
        this.remoteDesktopService.setEnableAutoClipboard(enable);
    }

    private async saveRemoteClipboardData(): Promise<boolean> {
        return await this.clipboardService.saveRemoteClipboardData();
    }

    private async sendClipboardData(): Promise<boolean> {
        return await this.clipboardService.sendClipboardData();
    }

    private invokeExtension(ext: Extension) {
        this.remoteDesktopService.invokeExtension(ext);
    }

    getExposedFunctions(): UserInteraction {
        return {
            setVisibility: this.setVisibility.bind(this),
            configBuilder: this.configBuilder.bind(this),
            connect: this.connect.bind(this),
            setScale: this.setScale.bind(this),
            onSessionEvent: (callback) => {
                this.remoteDesktopService.sessionEventObservable.subscribe(callback);
            },
            ctrlAltDel: this.ctrlAltDel.bind(this),
            metaKey: this.metaKey.bind(this),
            shutdown: this.shutdown.bind(this),
            setKeyboardUnicodeMode: this.setKeyboardUnicodeMode.bind(this),
            setCursorStyleOverride: this.setCursorStyleOverride.bind(this),
            resize: this.resize.bind(this),
            setEnableClipboard: this.setEnableClipboard.bind(this),
            setEnableAutoClipboard: this.setEnableAutoClipboard.bind(this),
            saveRemoteClipboardData: this.saveRemoteClipboardData.bind(this),
            sendClipboardData: this.sendClipboardData.bind(this),
            invokeExtension: this.invokeExtension.bind(this),
        };
    }
}
