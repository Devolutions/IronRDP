import type { SessionEventType } from '../enums/SessionEventType';

export enum RemoteDesktopErrorKind {
    General = 0,
    WrongPassword = 1,
    LogonFailure = 2,
    AccessDenied = 3,
    RDCleanPath = 4,
    ProxyConnect = 5,
}
export interface RemoteDesktopError {
    backtrace: () => string;
    kind: () => RemoteDesktopErrorKind;
}

export interface SessionEvent {
    type: SessionEventType;
    data: RemoteDesktopError | string;
}
