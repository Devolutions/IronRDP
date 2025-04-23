import type { DesktopSize } from './DesktopSize';

export interface NewSessionInfo {
    sessionId: number;
    websocketPort: number;
    initialDesktopSize: DesktopSize;
}
