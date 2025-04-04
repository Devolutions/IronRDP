export interface ServerRect {
    clone_buffer(): Uint8Array;

    bottom: number;
    left: number;
    right: number;
    top: number;
}
