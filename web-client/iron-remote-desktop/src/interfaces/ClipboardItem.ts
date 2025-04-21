export interface ClipboardItem {
    mime_type(): string;
    value(): unknown;
}
