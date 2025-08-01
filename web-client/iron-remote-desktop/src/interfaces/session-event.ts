import type { SessionEventType } from '../enums/SessionEventType';

export enum IronErrorKind {
    General = 0,
    WrongPassword = 1,
    LogonFailure = 2,
    AccessDenied = 3,
    RDCleanPath = 4,
    ProxyConnect = 5,
    NegotiationFailure = 6,
}

export interface IronError {
    backtrace: () => string;
    kind: () => IronErrorKind;
}

export interface SessionEvent {
    type: SessionEventType;
    data: IronError | string;
}
