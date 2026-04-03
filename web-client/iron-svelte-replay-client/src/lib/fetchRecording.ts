type milliseconds = number;

import { HEADER_SIZE, INDEX_ROW_SIZE, MAX_PDUS, SUPPORTED_VERSION, toSafeNumber } from './recordingFormat.js';

/** Error thrown when the server returns a non-2xx HTTP status code.
 * Carries the status code so upstream catch blocks can distinguish auth
 * failures (401/403) from server errors (5xx) without string parsing.
 */
export class FetchHttpError extends Error {
    constructor(public readonly httpStatus: number) {
        super(`HTTP error ${httpStatus}`);
        this.name = 'FetchHttpError';
    }
}

/** Static or callback form of RequestInit for auth / custom headers.
 *
 * - Object form: `{ headers: { Authorization: 'Bearer ...' } }` — evaluated once.
 * - Callback form: `() => { headers: ... }` — called fresh on every fetch, allowing
 *   transparent token rotation without reloading the player.
 * - Async callback: `async () => { headers: ... }` — same, but can await a token refresh.
 *
 * `method` and `body` are excluded — they are meaningless for byte-range fetches.
 * The `Range` header is always set by the fetch layer and cannot be overridden.
 */
export type FetchOptions =
    | Omit<RequestInit, 'method' | 'body'>
    | (() => Omit<RequestInit, 'method' | 'body'> | Promise<Omit<RequestInit, 'method' | 'body'>>);

/** Normalize HeadersInit to a plain Record<string, string> so it can be safely spread.
 * HeadersInit can be a Headers instance, string[][], or Record<string, string>.
 * Spreading a Headers instance or string[][] directly produces incorrect results.
 */
function normalizeHeaders(headers: HeadersInit): Record<string, string> {
    if (headers instanceof Headers) {
        const result: Record<string, string> = {};
        headers.forEach((value, key) => {
            result[key] = value;
        });
        return result;
    }
    if (Array.isArray(headers)) {
        return Object.fromEntries(headers);
    }
    return headers as Record<string, string>;
}

/** Resolve FetchOptions to a normalized form, invoking the callback if needed.
 * Returned headers are always a plain Record<string, string> safe to spread.
 */
export async function resolveFetchOptions(
    fetchOptions?: FetchOptions,
): Promise<Omit<RequestInit, 'method' | 'body'> & { headers: Record<string, string> }> {
    const resolved = (() => {
        if (fetchOptions === undefined) return {};
        if (typeof fetchOptions === 'function') return fetchOptions();
        return fetchOptions;
    })();
    const { headers, ...rest } = await resolved;
    return { ...rest, headers: headers ? normalizeHeaders(headers) : {} };
}

export interface Header {
    version: number;
    duration: milliseconds;
    totalPdus: number;
}

export interface IndexTableRow {
    timeOffset: milliseconds;
    pduLength: number;
    byteOffset: bigint;
    direction: number; // 0 = Client, 1 = Server
}

export async function fetchFileRanges(
    url: string,
    startBytes: number,
    endBytes: number,
    fetchOptions?: FetchOptions,
    signal?: AbortSignal,
): Promise<ArrayBuffer> {
    const resolved = await resolveFetchOptions(fetchOptions);
    const response = await fetch(url, {
        ...resolved,
        headers: { ...resolved.headers, Range: `bytes=${startBytes}-${endBytes}` },
        signal,
    });

    if (response.status === 200) {
        // The server ignored the Range header and returned the full file.
        // Even when startBytes is 0 the response is wrong — subsequent fetches
        // for non-zero offsets would fail, so reject early with a clear message.
        throw new Error(
            `server returned 200 instead of 206 Partial Content for a Range request, ` +
                `the server must support HTTP Range requests`,
        );
    }

    if (response.status !== 206) {
        throw new FetchHttpError(response.status);
    }

    if (!response.headers.has('content-range')) {
        throw new Error(
            `server returned 206 without a Content-Range header, ` +
                `the server may not support HTTP Range requests correctly`,
        );
    }

    return response.arrayBuffer();
}

function parseHeader(buffer: ArrayBuffer): Header {
    const view = new DataView(buffer);

    const version = view.getUint32(0, false);
    if (version !== SUPPORTED_VERSION) {
        throw new Error(`unsupported recording version ${version}, expected ${SUPPORTED_VERSION}`);
    }
    const totalPdus = toSafeNumber(view.getBigUint64(4, false), 'totalPdus');
    const duration = toSafeNumber(view.getBigUint64(12, false), 'duration');
    return { version, totalPdus, duration };
}

function parseIndexTable(buffer: ArrayBuffer, startOffset: number, count: number): IndexTableRow[] {
    const view = new DataView(buffer);
    const entries: IndexTableRow[] = [];

    for (let i = 0; i < count; i++) {
        const entryOffset = startOffset + i * INDEX_ROW_SIZE;

        const timeOffset = view.getUint32(entryOffset, false);
        const pduLength = view.getUint32(entryOffset + 4, false);
        const byteOffset = view.getBigUint64(entryOffset + 8, false);
        const direction = view.getUint8(entryOffset + 16);

        entries.push({ timeOffset, pduLength, byteOffset, direction });
    }

    return entries;
}

export async function fetchHeader(url: string, fetchOptions?: FetchOptions, signal?: AbortSignal): Promise<Header> {
    const buffer = await fetchFileRanges(url, 0, HEADER_SIZE - 1, fetchOptions, signal);
    return parseHeader(buffer);
}

export async function fetchIndexTable(
    url: string,
    totalPDUs: number,
    fetchOptions?: FetchOptions,
    signal?: AbortSignal,
): Promise<IndexTableRow[]> {
    if (totalPDUs === 0) {
        return [];
    }
    if (totalPDUs > MAX_PDUS) {
        throw new Error(
            `recording claims ${totalPDUs.toLocaleString()} PDUs; exceeds maximum of ${MAX_PDUS.toLocaleString()}, ` +
                `the recording file may be corrupt`,
        );
    }
    const endBytes = HEADER_SIZE + INDEX_ROW_SIZE * totalPDUs - 1;
    const buffer = await fetchFileRanges(url, HEADER_SIZE, endBytes, fetchOptions, signal);
    return parseIndexTable(buffer, 0, totalPDUs);
}
