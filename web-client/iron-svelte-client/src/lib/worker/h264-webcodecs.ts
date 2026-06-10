/// <reference lib="dom" />
//
// Hardware H.264 decode via the browser WebCodecs `VideoDecoder`, for the EGFX/AVC420 path.
//
// EGFX `RFX_AVC420_BITMAP_STREAM` carries H.264 in **AVC format** (4-byte big-endian length-prefixed
// NAL units, not Annex B). WebCodecs accepts that format directly when the decoder is configured
// with an `avcC` `description`; we build that `description` from the SPS/PPS found in the first
// frames. Decoded `VideoFrame`s are delivered via the `onFrame` callback; the consumer uploads them
// to the GPU surface (softblit) and must `close()` each frame.
//
// WebCodecs runs in a Web Worker, so this lives alongside `rdp.worker.ts`.

const NAL_SPS = 7;
const NAL_PPS = 8;
const NAL_IDR = 5;

/** Split an AVC-format (4-byte BE length-prefixed) buffer into individual NAL unit slices. */
function splitAvcNals(data: Uint8Array): Uint8Array[] {
    const nals: Uint8Array[] = [];
    let offset = 0;
    while (offset + 4 <= data.length) {
        const len = (data[offset]! << 24) | (data[offset + 1]! << 16) | (data[offset + 2]! << 8) | data[offset + 3]!;
        offset += 4;
        if (len <= 0 || offset + len > data.length) {
            break;
        }
        nals.push(data.subarray(offset, offset + len));
        offset += len;
    }
    return nals;
}

function nalType(nal: Uint8Array): number {
    return (nal[0] ?? 0) & 0x1f;
}

/**
 * Build an `avcC` (AVCDecoderConfigurationRecord) from one SPS and one PPS, as WebCodecs expects in
 * `VideoDecoderConfig.description` for AVC-format input. Layout per ISO/IEC 14496-15.
 */
function buildAvcC(sps: Uint8Array, pps: Uint8Array): Uint8Array {
    const out: number[] = [];
    out.push(1); // configurationVersion
    out.push(sps[1] ?? 0x42); // AVCProfileIndication
    out.push(sps[2] ?? 0); // profile_compatibility
    out.push(sps[3] ?? 0x1e); // AVCLevelIndication
    out.push(0xff); // 6 bits reserved + lengthSizeMinusOne = 3 (4-byte lengths)
    out.push(0xe1); // 3 bits reserved + numOfSequenceParameterSets = 1
    out.push((sps.length >> 8) & 0xff, sps.length & 0xff);
    out.push(...sps);
    out.push(1); // numOfPictureParameterSets = 1
    out.push((pps.length >> 8) & 0xff, pps.length & 0xff);
    out.push(...pps);
    return new Uint8Array(out);
}

function avcCodecString(sps: Uint8Array): string {
    const profile = sps[1] ?? 0x42;
    const compat = sps[2] ?? 0x00;
    const level = sps[3] ?? 0x1e;
    const hex = (n: number) => n.toString(16).padStart(2, '0');
    return `avc1.${hex(profile)}${hex(compat)}${hex(level)}`;
}

export interface H264WebCodecsCallbacks {
    onFrame: (frame: VideoFrame) => void;
    onError: (message: string) => void;
}

/**
 * Stateful AVC420 → `VideoFrame` decoder. Feed each frame's AVC payload with `decode()`; decoded
 * frames arrive (in order, asynchronously) on `onFrame`. Configures lazily once SPS+PPS are seen.
 */
export class H264WebCodecsDecoder {
    private decoder?: VideoDecoder;
    private sps?: Uint8Array;
    private pps?: Uint8Array;
    private configured = false;
    private timestamp = 0;

    constructor(private cb: H264WebCodecsCallbacks) {}

    static isSupported(): boolean {
        return typeof VideoDecoder !== 'undefined';
    }

    /** Feed one AVC-format frame (4-byte length-prefixed NALs) from an AVC420 bitmap stream. */
    decode(data: Uint8Array): void {
        const nals = splitAvcNals(data);
        let hasIdr = false;
        for (const nal of nals) {
            const t = nalType(nal);
            if (t === NAL_SPS) {
                this.sps = nal.slice();
            } else if (t === NAL_PPS) {
                this.pps = nal.slice();
            } else if (t === NAL_IDR) {
                hasIdr = true;
            }
        }

        if (!this.configured) {
            if (this.sps == null || this.pps == null) {
                // Can't configure until the first SPS/PPS arrive; drop frames until then.
                return;
            }
            this.configure(this.sps, this.pps);
        }

        if (this.decoder == null) {
            return;
        }

        try {
            const chunk = new EncodedVideoChunk({
                type: hasIdr ? 'key' : 'delta',
                timestamp: this.timestamp,
                data,
            });
            this.timestamp += 1;
            this.decoder.decode(chunk);
        } catch (e: unknown) {
            this.cb.onError(`H.264 decode submit failed: ${e instanceof Error ? e.message : String(e)}`);
        }
    }

    private configure(sps: Uint8Array, pps: Uint8Array): void {
        const decoder = new VideoDecoder({
            output: (frame) => this.cb.onFrame(frame),
            error: (e) => this.cb.onError(`VideoDecoder error: ${e.message}`),
        });
        decoder.configure({
            codec: avcCodecString(sps),
            description: buildAvcC(sps, pps),
            optimizeForLatency: true,
        });
        this.decoder = decoder;
        this.configured = true;
    }

    reset(): void {
        try {
            this.decoder?.close();
        } catch {
            /* ignore */
        }
        this.decoder = undefined;
        this.configured = false;
        this.sps = undefined;
        this.pps = undefined;
        this.timestamp = 0;
    }
}
