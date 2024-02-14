﻿import type { ScreenScale } from '../enums/ScreenScale';
import type { Observable } from 'rxjs';
import type { NewSessionInfo } from './NewSessionInfo';
import type { SessionEvent } from './session-event';
import type { DesktopSize } from './DesktopSize';

export interface UserInteraction {
    setVisibility(state: boolean): void;

    setScale(scale: ScreenScale): void;

    connect(
        username: string,
        password: string,
        destination: string,
        proxyAddress: string,
        serverDomain: string,
        authToken: string,
        desktopSize?: DesktopSize,
        preConnectionBlob?: string,
        kdc_proxy_url?: string,
    ): Observable<NewSessionInfo>;

    setKeyboardUnicodeMode(use_unicode: boolean): void;

    ctrlAltDel(): void;

    metaKey(): void;

    shutdown(): void;

    sessionListener: Observable<SessionEvent>;
}
