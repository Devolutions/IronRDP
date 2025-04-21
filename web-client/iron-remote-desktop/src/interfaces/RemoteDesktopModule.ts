import type { DesktopSize } from './DesktopSize';
import type { DeviceEvent } from './DeviceEvent';
import type { InputTransaction } from './InputTransaction';
import type { IronError } from './session-event';
import type { Session } from './Session';
import type { SessionBuilder } from './SessionBuilder';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardData } from './ClipboardData';
import type { ClipboardItem } from './ClipboardItem';

export interface RemoteDesktopModule {
    init: () => Promise<unknown>;
    setup: (logLevel: string) => void;
    DesktopSize: DesktopSize;
    DeviceEvent: DeviceEvent;
    InputTransaction: InputTransaction;
    RemoteDesktopError: IronError;
    Session: Session;
    SessionBuilder: SessionBuilder;
    SessionTerminationInfo: SessionTerminationInfo;
    ClipboardData: ClipboardData;
    ClipboardItem: ClipboardItem;
}
