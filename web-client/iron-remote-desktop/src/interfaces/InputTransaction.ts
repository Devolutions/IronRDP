import type { DeviceEvent } from './DeviceEvent';

export interface InputTransaction {
    free(): void;
    // eslint-disable-next-line @typescript-eslint/no-misused-new
    new (): InputTransaction;
    add_event(event: DeviceEvent): void;
}
