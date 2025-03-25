import type { DeviceEvent } from './DeviceEvent';

export interface InputTransaction {
    // eslint-disable-next-line @typescript-eslint/no-misused-new
    new (): InputTransaction;
    add_event(event: DeviceEvent): void;
    free(): void;
}
