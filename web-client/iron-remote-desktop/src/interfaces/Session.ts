import type { InputTransaction } from './InputTransaction';
import type { DesktopSize } from './DesktopSize';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardTransaction } from './ClipboardTransaction';

export interface Session {
    run(): Promise<SessionTerminationInfo>;
    desktop_size(): DesktopSize;
    apply_inputs(transaction: InputTransaction): void;
    release_all_inputs(): void;
    synchronize_lock_keys(scroll_lock: boolean, num_lock: boolean, caps_lock: boolean, kana_lock: boolean): void;
    extension_call(ident: string, params: unknown): unknown;
    shutdown(): void;
    on_clipboard_paste(content: ClipboardTransaction): Promise<void>;
    resize(
        width: number,
        height: number,
        scale_factor?: number | null,
        physical_width?: number | null,
        physical_height?: number | null,
    ): void;
    supports_unicode_keyboard_shortcuts(): boolean;
    free(): void;
}
