import type { ClipboardItem } from './ClipboardItem';

export interface ClipboardData {
    addText(mimeType: string, text: string): void;
    addBinary(mimeType: string, binary: Uint8Array): void;
    items(): ClipboardItem[];
    isEmpty(): boolean;
}
