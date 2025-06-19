import type { DeviceEvent } from './DeviceEvent';

export interface InputTransaction {
    addEvent(event: DeviceEvent): void;
}
