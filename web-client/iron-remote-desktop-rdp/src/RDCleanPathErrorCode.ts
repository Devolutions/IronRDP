/**
 * RDCleanPath error codes reported by `RDCleanPathDetails.errorCode`.
 *
 * Same values as the constants of the `ironrdp-rdcleanpath` crate, whose README specifies them.
 * Codes 16384 to 32767 are implementation-specific: their meaning depends on the proxy.
 *
 * Unlike `FileContentsFlags`, no union type of these values is exported: the code space is open,
 * so `errorCode` stays a `number` and may carry codes not listed here.
 */
export const RDCleanPathErrorCode = {
    /** General error, without a more specific reason. */
    GENERAL: 1,

    /** The proxy credential has expired. */
    EXPIRED: 100,

    /** The proxy credential is not valid yet. */
    NOT_YET_VALID: 101,

    /** The proxy credential has been revoked. */
    REVOKED: 102,

    /** The proxy could not connect to the target. */
    TARGET_UNREACHABLE: 103,

    /** The proxy could not resolve the target host name. */
    DNS_RESOLUTION_FAILED: 104,

    /** The TLS handshake between the proxy and the target failed. */
    TLS_HANDSHAKE_FAILED: 105,
} as const;
