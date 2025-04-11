import type { DeviceEvent } from './DeviceEvent';

export interface InputTransaction {
    construct(): InputTransaction;
    add_event(event: DeviceEvent): void;
}
