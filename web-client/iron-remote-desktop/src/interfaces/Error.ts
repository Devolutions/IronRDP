export enum IronErrorKind {
    General = 0,
    WrongPassword = 1,
    LogonFailure = 2,
    AccessDenied = 3,
    RDCleanPath = 4,
    ProxyConnect = 5,
    NegotiationFailure = 6,
}

export interface RDCleanPathDetails {
    readonly httpStatusCode?: number;
    readonly wsaErrorCode?: number;
    readonly tlsAlertCode?: number;
    /** RDCleanPath error code; codes 16384 to 32767 are specific to the proxy implementation. */
    readonly errorCode?: number;
    /** Stable snake_case name of the standard reason the proxy rejected the connection, such as `expired`. */
    readonly reason?: string;
}

export interface IronError {
    backtrace: () => string;
    kind: () => IronErrorKind;
    rdcleanpathDetails: () => RDCleanPathDetails | undefined;
}
