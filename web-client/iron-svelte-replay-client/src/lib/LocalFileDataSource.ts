import type { PduDirection, ReplayDataSource, ReplayMetadata, ReplayPdu } from './ReplayDataSource.types.js';
import { parseHeader, parseIndexTable, type IndexTableRow } from './fetchRecording.js';
import { HEADER_SIZE, INDEX_ROW_SIZE, MAX_PDUS, searchByTime, toSafeNumber } from './recordingFormat.js';

export class LocalFileDataSource implements ReplayDataSource {
    private source: File | Blob | ArrayBuffer;
    private buffer?: ArrayBuffer;
    private indexTable?: IndexTableRow[];
    private totalPdus = 0;
    private durationMs = 0;

    constructor(source: File | Blob | ArrayBuffer) {
        this.source = source;
    }

    async open(_signal?: AbortSignal): Promise<ReplayMetadata> {
        this.close();
        const buffer = this.source instanceof ArrayBuffer ? this.source : await this.source.arrayBuffer();
        if (buffer.byteLength < HEADER_SIZE) throw new Error('recording header extends beyond file boundary');
        const header = parseHeader(buffer);
        if (header.totalPdus > MAX_PDUS) {
            throw new Error(
                `recording claims ${header.totalPdus.toLocaleString()} PDUs; exceeds maximum of ${MAX_PDUS.toLocaleString()}, ` +
                    `the recording file may be corrupt`,
            );
        }

        const indexEnd = HEADER_SIZE + INDEX_ROW_SIZE * header.totalPdus;
        if (indexEnd > buffer.byteLength) {
            throw new Error(
                `index table extends beyond file boundary (ends at ${indexEnd}, file size ${buffer.byteLength})`,
            );
        }
        const indexTable = parseIndexTable(buffer, HEADER_SIZE, header.totalPdus);
        for (const [i, entry] of indexTable.entries()) {
            const byteOffset = toSafeNumber(entry.byteOffset, 'byteOffset');
            if (byteOffset + entry.pduLength > buffer.byteLength) {
                throw new Error(
                    `PDU ${i} extends beyond file boundary (offset ${byteOffset}, length ${entry.pduLength}, file size ${buffer.byteLength})`,
                );
            }
        }

        this.buffer = buffer;
        this.indexTable = indexTable;
        this.totalPdus = header.totalPdus;
        this.durationMs = header.duration > 0 ? header.duration : (indexTable[indexTable.length - 1]?.timeOffset ?? 0);

        return {
            durationMs: this.durationMs,
            totalPdus: this.totalPdus,
        };
    }

    async fetch(fromMs: number, toMs: number, _signal?: AbortSignal): Promise<ReplayPdu[]> {
        if (!this.buffer || !this.indexTable) return [];

        const startIdx = searchByTime(this.indexTable, fromMs);
        if (startIdx >= this.indexTable.length) return [];

        const pdus: ReplayPdu[] = [];
        const view = new Uint8Array(this.buffer);

        for (let i = startIdx; i < this.indexTable.length; i++) {
            const entry = this.indexTable[i];
            if (entry.timeOffset >= toMs) break;
            const offset = toSafeNumber(entry.byteOffset, 'byteOffset');
            pdus.push({
                timestampMs: entry.timeOffset,
                source: entry.direction as PduDirection,
                data: view.subarray(offset, offset + entry.pduLength),
            });
        }

        return pdus;
    }

    close(): void {
        this.buffer = undefined;
        this.indexTable = undefined;
    }
}
