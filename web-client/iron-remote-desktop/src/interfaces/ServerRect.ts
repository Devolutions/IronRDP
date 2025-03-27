export interface ServerRect {
    bottom: number;
    left: number;
    right: number;
    top: number;

    clone_buffer(): Uint8Array;
    free(): void;
}
