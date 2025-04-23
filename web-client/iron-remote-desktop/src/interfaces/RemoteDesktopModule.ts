import type { DesktopSize } from './DesktopSize';
import type { DeviceEvent } from './DeviceEvent';
import type { InputTransaction } from './InputTransaction';
import type { SessionBuilder } from './SessionBuilder';
import type { ClipboardData } from './ClipboardData';

export interface RemoteDesktopModule {
    createDesktopSize(width: number, height: number): DesktopSize;
    createMouseButtonPressed(button: number): DeviceEvent;
    createMouseButtonReleased(button: number): DeviceEvent;
    createMouseMove(x: number, y: number): DeviceEvent;
    createWheelRotations(vertical: boolean, rotation_units: number): DeviceEvent;
    createKeyPressed(scancode: number): DeviceEvent;
    createKeyReleased(scancode: number): DeviceEvent;
    createUnicodePressed(unicode: string): DeviceEvent;
    createUnicodeReleased(unicode: string): DeviceEvent;
    createInputTransaction(): InputTransaction;
    createSessionBuilder(): SessionBuilder;
    createClipboardData(): ClipboardData;
}
