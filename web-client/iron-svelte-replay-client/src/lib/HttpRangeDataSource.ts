import type { PduDirection, ReplayDataSource, ReplayMetadata, ReplayPdu } from './ReplayDataSource.types.js';
import {
    fetchHeader,
    fetchIndexTable,
    fetchFileRanges,
    type FetchOptions,
    type Header,
    type IndexTableRow,
} from './fetchRecording.js';
import { searchByTime, toSafeNumber } from './recordingFormat.js';

export class HttpRangeDataSource implements ReplayDataSource {
    private url: string;
    private fetchOptions?: FetchOptions;
    private header?: Header;
    private indexTable?: IndexTableRow[];

    constructor(url: string, fetchOptions?: FetchOptions) {
        this.url = url;
        this.fetchOptions = fetchOptions;
    }

    async open(signal?: AbortSignal): Promise<ReplayMetadata> {
        this.header = await fetchHeader(this.url, this.fetchOptions, signal);
        this.indexTable = await fetchIndexTable(this.url, this.header.totalPdus, this.fetchOptions, signal);

        const lastEntry = this.indexTable[this.indexTable.length - 1];
        const duration = this.header.duration > 0 ? this.header.duration : (lastEntry?.timeOffset ?? 0);

        return {
            durationMs: duration,
            totalPdus: this.header.totalPdus,
        };
    }

    async fetch(fromMs: number, toMs: number, signal?: AbortSignal): Promise<ReplayPdu[]> {
        if (!this.indexTable) return [];

        const startIdx = searchByTime(this.indexTable, fromMs);
        if (startIdx >= this.indexTable.length) return [];

        let endIdx = startIdx;
        while (endIdx < this.indexTable.length && this.indexTable[endIdx].timeOffset < toMs) {
            endIdx++;
        }
        if (endIdx === startIdx) return [];

        const firstEntry = this.indexTable[startIdx];
        const lastEntry = this.indexTable[endIdx - 1];
        const startByte = toSafeNumber(firstEntry.byteOffset, 'byteOffset');
        const endByte = toSafeNumber(lastEntry.byteOffset, 'byteOffset') + lastEntry.pduLength - 1;

        const buffer = await fetchFileRanges(this.url, startByte, endByte, this.fetchOptions, signal);

        const pdus: ReplayPdu[] = [];
        const baseOffset = toSafeNumber(firstEntry.byteOffset, 'byteOffset');
        const view = new Uint8Array(buffer);

        for (let i = startIdx; i < endIdx; i++) {
            const entry = this.indexTable[i];
            const offset = toSafeNumber(entry.byteOffset, 'byteOffset') - baseOffset;
            pdus.push({
                timestampMs: entry.timeOffset,
                source: entry.direction as PduDirection,
                data: view.subarray(offset, offset + entry.pduLength),
            });
        }

        return pdus;
    }

    close(): void {}
}
