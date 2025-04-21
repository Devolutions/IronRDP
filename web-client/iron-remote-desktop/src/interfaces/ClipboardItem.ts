export interface ClipboardItem {
    mime_type(): string;
    value(): string | Uint8Array;
}
