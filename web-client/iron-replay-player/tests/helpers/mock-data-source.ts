import { vi } from 'vitest';
import type {
    ReplayDataSource,
    ReplayPdu,
    ReplayMetadata,
    PduDirection,
} from '../../src/interfaces/ReplayDataSource.js';

export interface MockDataSourceOptions {
    durationMs: number;
    totalPdus?: number;
    initialWidth?: number;
    initialHeight?: number;
    /** When true, open() returns a pending promise resolved via resolveOpen(). */
    deferOpen?: boolean;
}

interface PendingRequest<T> {
    resolve: (value: T) => void;
    reject: (error: Error) => void;
}

export interface PendingFetch extends PendingRequest<ReplayPdu[]> {
    fromMs: number;
    toMs: number;
}

export function createMockDataSource(options: MockDataSourceOptions) {
    const metadata: ReplayMetadata = {
        durationMs: options.durationMs,
        totalPdus: options.totalPdus ?? 0,
        initialWidth: options.initialWidth,
        initialHeight: options.initialHeight,
    };

    const pendingFetches: PendingFetch[] = [];
    let pendingOpen: PendingRequest<ReplayMetadata> | null = null;

    function wireAbort<T>(signal: AbortSignal | undefined, pending: PendingRequest<T>): void {
        signal?.addEventListener('abort', () => {
            pending.reject(new DOMException('Aborted', 'AbortError'));
        });
    }

    const dataSource: ReplayDataSource = {
        open: vi.fn((signal?: AbortSignal) => {
            if (options.deferOpen === true) {
                return new Promise<ReplayMetadata>((resolve, reject) => {
                    pendingOpen = { resolve, reject };
                    wireAbort(signal, pendingOpen);
                });
            }
            return Promise.resolve(metadata);
        }),

        fetch: vi.fn((fromMs: number, toMs: number, signal?: AbortSignal) => {
            return new Promise<ReplayPdu[]>((resolve, reject) => {
                const entry: PendingFetch = { fromMs, toMs, resolve, reject };
                pendingFetches.push(entry);
                wireAbort(signal, entry);
            });
        }),

        close: vi.fn(),
    };

    return {
        dataSource,

        /** Resolve the oldest pending fetch with the given PDUs. */
        resolveFetch(pdus: ReplayPdu[] = []): void {
            const entry = pendingFetches.shift();
            if (!entry) throw new Error('No pending fetch to resolve');
            entry.resolve(pdus);
        },

        /** Reject the oldest pending fetch. */
        rejectFetch(error: Error = new Error('fetch failed')): void {
            const entry = pendingFetches.shift();
            if (!entry) throw new Error('No pending fetch to reject');
            entry.reject(error);
        },

        /** Number of pending (unresolved) fetches. */
        get pendingCount(): number {
            return pendingFetches.length;
        },

        /** Peek at the oldest pending fetch's range. */
        get oldestPending(): PendingFetch | undefined {
            return pendingFetches[0];
        },

        /** Resolve the deferred open() call. Only usable when deferOpen: true. */
        resolveOpen(): void {
            if (!pendingOpen) throw new Error('No pending open to resolve (deferOpen not set?)');
            pendingOpen.resolve(metadata);
            pendingOpen = null;
        },

        /** Reject the deferred open() call. Only usable when deferOpen: true. */
        rejectOpen(error: Error = new Error('open failed')): void {
            if (!pendingOpen) throw new Error('No pending open to reject (deferOpen not set?)');
            pendingOpen.reject(error);
            pendingOpen = null;
        },
    };
}

/** Create a single PDU for test data. */
export function makePdu(timestampMs: number, source: PduDirection = 1): ReplayPdu {
    return {
        timestampMs,
        source,
        data: new Uint8Array([0]),
    };
}

/** Generate PDUs at regular intervals within [fromMs, toMs). */
export function makePdus(fromMs: number, toMs: number, intervalMs: number = 100): ReplayPdu[] {
    const pdus: ReplayPdu[] = [];
    for (let t = fromMs; t < toMs; t += intervalMs) {
        pdus.push(makePdu(t));
    }
    return pdus;
}
