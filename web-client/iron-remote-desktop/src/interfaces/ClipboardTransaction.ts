import type { ClipboardContent } from './ClipboardContent';

export interface ClipboardTransaction {
    // eslint-disable-next-line @typescript-eslint/no-misused-new
    new (): ClipboardTransaction;
    add_content(content: ClipboardContent): void;
    is_empty(): boolean;
    content(): Array<unknown>;
    free(): void;
}
