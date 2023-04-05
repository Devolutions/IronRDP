import type {DesktopSize} from './DesktopSize';

export interface ResizeEvent {
    session_id: number,
    desktop_size: DesktopSize,
}
