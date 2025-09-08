import type { DesktopSize } from './DesktopSize';
import type { SessionTerminationInfo } from './SessionTerminationInfo';

export interface NewSessionInfo {
    sessionId: number;
    websocketPort: number;
    initialDesktopSize: DesktopSize;
    run: () => Promise<SessionTerminationInfo>;
}
