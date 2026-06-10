// Minimal ambient declarations for the WebCodecs APIs used by `h264-webcodecs.ts`.
// The project's TS lib doesn't include WebCodecs; we declare only what we use rather than pull in
// `@types/dom-webcodecs`. Available at runtime in Chromium/Safari (and in Web Workers).

declare global {
    type EncodedVideoChunkType = 'key' | 'delta';

    interface EncodedVideoChunkInit {
        type: EncodedVideoChunkType;
        timestamp: number;
        data: BufferSource;
    }

    class EncodedVideoChunk {
        constructor(init: EncodedVideoChunkInit);
        readonly type: EncodedVideoChunkType;
        readonly timestamp: number;
    }

    interface VideoFrame {
        readonly displayWidth: number;
        readonly displayHeight: number;
        readonly codedWidth: number;
        readonly codedHeight: number;
        close(): void;
    }

    interface VideoDecoderConfig {
        codec: string;
        description?: BufferSource;
        optimizeForLatency?: boolean;
        codedWidth?: number;
        codedHeight?: number;
    }

    interface VideoDecoderInit {
        output: (frame: VideoFrame) => void;
        error: (error: DOMException) => void;
    }

    class VideoDecoder {
        constructor(init: VideoDecoderInit);
        readonly decodeQueueSize: number;
        configure(config: VideoDecoderConfig): void;
        decode(chunk: EncodedVideoChunk): void;
        flush(): Promise<void>;
        reset(): void;
        close(): void;
    }
}

export {};
