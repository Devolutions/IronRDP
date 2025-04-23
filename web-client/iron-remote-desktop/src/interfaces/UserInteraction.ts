import type { ScreenScale } from '../enums/ScreenScale';
import type { NewSessionInfo } from './NewSessionInfo';
import type { SessionEvent } from './session-event';
import { ConfigBuilder } from '../services/ConfigBuilder';
import type { Config } from '../services/Config';

export interface UserInteraction {
    setVisibility(state: boolean): void;

    setScale(scale: ScreenScale): void;

    configBuilder(): ConfigBuilder;

    connect(config: Config): Promise<NewSessionInfo>;

    setKeyboardUnicodeMode(useUnicode: boolean): void;

    ctrlAltDel(): void;

    metaKey(): void;

    shutdown(): void;

    setCursorStyleOverride(style: string | null): void;

    onSessionEvent(callback: (event: SessionEvent) => void): void;

    resize(width: number, height: number, scale?: number): void;

    setEnableClipboard(enable: boolean): void;
}
