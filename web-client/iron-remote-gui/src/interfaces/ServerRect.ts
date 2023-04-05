export interface ServerRect {
    free(): void;

    clone_buffer(): Uint8Array;

    bottom: number;
    left: number;
    right: number;
    top: number;
}
