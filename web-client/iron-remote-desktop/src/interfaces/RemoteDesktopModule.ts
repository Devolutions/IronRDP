import type { DesktopSize } from './DesktopSize';
import type { DeviceEvent } from './DeviceEvent';
import type { InputTransaction } from './InputTransaction';
import type { IronError } from './session-event';
import type { Session } from './Session';
import type { SessionBuilder } from './SessionBuilder';
import type { SessionTerminationInfo } from './SessionTerminationInfo';
import type { ClipboardTransaction } from './ClipboardTransaction';
import type { ClipboardContent } from './ClipboardContent';

export interface RemoteDesktopModule {
    init: () => Promise<unknown>;
    iron_init: (logLevel: string) => void;
    DesktopSize: DesktopSize;
    DeviceEvent: DeviceEvent;
    InputTransaction: InputTransaction;
    RemoteDesktopError: IronError;
    Session: Session;
    SessionBuilder: SessionBuilder;
    SessionTerminationInfo: SessionTerminationInfo;
    ClipboardTransaction: ClipboardTransaction;
    ClipboardContent: ClipboardContent;
}
