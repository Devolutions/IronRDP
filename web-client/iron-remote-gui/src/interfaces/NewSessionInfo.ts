import type { DesktopSize } from './DesktopSize';

export interface NewSessionInfo {
    session_id: number;
    websocket_port: number;
    initial_desktop_size: DesktopSize;
}
