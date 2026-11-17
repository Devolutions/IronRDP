import { afterEach, describe, expect, it, vi } from 'vitest';
import { HttpRangeDataSource } from '../../iron-svelte-replay-client/src/lib/HttpRangeDataSource.js';
import { fetchFileRanges } from '../../iron-svelte-replay-client/src/lib/fetchRecording.js';

function recording(lengths: number[]): Uint8Array {
    const dataStart = 20 + lengths.length * 17;
    const bytes = new Uint8Array(dataStart + lengths.reduce((sum, length) => sum + length, 0));
    const view = new DataView(bytes.buffer);
    view.setUint32(0, 1);
    view.setBigUint64(4, BigInt(lengths.length));
    let offset = dataStart;
    lengths.forEach((length, i) => {
        const row = 20 + i * 17;
        view.setUint32(row, i);
        view.setUint32(row + 4, length);
        view.setBigUint64(row + 8, BigInt(offset));
        view.setUint8(row + 16, 1);
        bytes.fill(i % 256, offset, offset + length);
        offset += length;
    });
    return bytes;
}

function serve(bytes: Uint8Array): string[] {
    const ranges: string[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (_url: string, init: RequestInit) => {
            const range = (init.headers as Record<string, string>).Range;
            ranges.push(range);
            const [, start, end] = /^bytes=(\d+)-(\d+)$/.exec(range)!;
            return new Response(bytes.slice(Number(start), Number(end) + 1), {
                status: 206,
                headers: { 'Content-Range': `bytes ${start}-${end}/${bytes.length}` },
            });
        }),
    );
    return ranges;
}

afterEach(() => vi.unstubAllGlobals());

describe('HTTP replay ranges', () => {
    it('rejects an oversized index entry before fetching PDU data', async () => {
        const bytes = recording([1]);
        new DataView(bytes.buffer).setUint32(24, 0xffff_ffff);
        const ranges = serve(bytes);
        const source = new HttpRangeDataSource('https://example.test/recording');

        await expect(source.open()).rejects.toThrow('invalid PDU length');
        expect(ranges).toHaveLength(2);
    });

    it('splits large valid data into bounded requests without changing PDUs', async () => {
        const bytes = recording(Array(130).fill(65_535));
        const ranges = serve(bytes);
        const source = new HttpRangeDataSource('https://example.test/recording');
        await source.open();
        const pdus = await source.fetch(0, 130);

        expect(pdus).toHaveLength(130);
        expect(pdus[129].data).toHaveLength(65_535);
        expect(pdus[129].data[0]).toBe(129);
        expect(ranges).toHaveLength(4);
        for (const range of ranges.slice(2)) {
            const [, start, end] = /^bytes=(\d+)-(\d+)$/.exec(range)!;
            expect(Number(end) - Number(start) + 1).toBeLessThanOrEqual(8 * 1024 * 1024);
        }
    });

    it('rejects a response whose body exceeds the requested range', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(
                async () =>
                    new Response(new Uint8Array(4), {
                        status: 206,
                        headers: { 'Content-Range': 'bytes 0-1/4' },
                    }),
            ),
        );
        await expect(fetchFileRanges('https://example.test/recording', 0, 1)).rejects.toThrow('more bytes');
    });
});
