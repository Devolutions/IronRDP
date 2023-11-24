import type { SessionEventType } from '../enums/SessionEventType';

export enum UserIronRdpErrorKind {
    General = 0,
    WrongPassword = 1,
    LogonFailure = 2,
    AccessDenied = 3,
    RDCleanPath = 4,
    ProxyConnect = 5,
}
export interface UserIronRdpError {
    backtrace: () => string;
    kind: () => UserIronRdpErrorKind;
}

export interface SessionEvent {
    type: SessionEventType;
    data: UserIronRdpError | string;
}
