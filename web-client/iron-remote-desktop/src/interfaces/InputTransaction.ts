import type { DeviceEvent } from './DeviceEvent';

export interface InputTransaction {
    init(): InputTransaction;
    add_event(event: DeviceEvent): void;
}
