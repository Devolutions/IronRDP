export interface ClipboardItem {
    mimeType(): string;
    value(): string | Uint8Array;
}
