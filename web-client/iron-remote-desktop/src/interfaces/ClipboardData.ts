import type { ClipboardItem } from './ClipboardItem';

export interface ClipboardData {
    add_text(mime_type: string, text: string): void;
    add_binary(mime_type: string, binary: Uint8Array): void;
    items(): ClipboardItem[];
    is_empty(): boolean;
}
