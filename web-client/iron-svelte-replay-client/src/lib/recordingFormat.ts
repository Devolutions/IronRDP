// Recording binary format constants and shared utilities.
//
// The recording is a self-contained binary file: a fixed-size header followed
// by an index table and then the raw PDU data. All multi-byte integers are
// big-endian. See README.md for the full format specification.

/** Size of the file header in bytes (version + totalPdus + duration). */
export const HEADER_SIZE = 20;

/** Size of a single index table row in bytes (timeOffset + pduLength + byteOffset + direction). */
export const INDEX_ROW_SIZE = 17;

/** Maximum number of PDUs we'll accept (~17 MB index table). */
export const MAX_PDUS = 1_000_000;

/** Only version we support. */
export const SUPPORTED_VERSION = 1;

const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

/** Convert a uint64 to number, throwing if precision would be lost. */
export function toSafeNumber(value: bigint, field: string): number {
    if (value > MAX_SAFE) {
        throw new Error(`${field} value ${value} exceeds Number.MAX_SAFE_INTEGER`);
    }
    return Number(value);
}

/**
 * Binary search for the first entry whose `timeOffset` is >= `targetMs`.
 *
 * Returns the index of the first matching entry, or `entries.length` if all
 * entries are before `targetMs`. Works with any array of objects that have
 * a `timeOffset` property.
 */
export function searchByTime(entries: readonly { timeOffset: number }[], targetMs: number): number {
    let low = 0;
    let high = entries.length;
    while (low < high) {
        const mid = (low + high) >>> 1;
        if (entries[mid].timeOffset < targetMs) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    return low;
}
