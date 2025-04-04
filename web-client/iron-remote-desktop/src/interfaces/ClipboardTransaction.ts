import type { ClipboardContent } from './ClipboardContent';

export interface ClipboardTransaction {
    construct(): ClipboardTransaction;
    add_content(content: ClipboardContent): void;
    is_empty(): boolean;
    content(): Array<ClipboardContent>;
}
