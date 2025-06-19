import type { DesktopSize } from './DesktopSize';

export interface ResizeEvent {
    sessionId: number;
    desktopSize: DesktopSize;
}
