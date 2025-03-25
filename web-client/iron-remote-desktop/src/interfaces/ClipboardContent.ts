export interface ClipboardContent {
    new_text(mime_type: string, text: string): ClipboardContent;
    new_binary(mime_type: string, binary: Uint8Array): ClipboardContent;
    mime_type(): string;
    value(): any;
    free(): void;
}
