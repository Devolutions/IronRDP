import type { DesktopSize } from './DesktopSize';
import type { DeviceEvent } from './DeviceEvent';
import type { InputTransaction } from './InputTransaction';
import type { SessionBuilder } from './SessionBuilder';
import type { ClipboardData } from './ClipboardData';
import type { ConfigParser } from './ConfigParser';

export interface RemoteDesktopModule {
    DesktopSize: { new (width: number, height: number): DesktopSize };
    InputTransaction: { new (): InputTransaction };
    SessionBuilder: { new (): SessionBuilder };
    ClipboardData: { new (): ClipboardData };
    DeviceEvent: {
        mouseButtonPressed(button: number): DeviceEvent;
        mouseButtonReleased(button: number): DeviceEvent;
        mouseMove(x: number, y: number): DeviceEvent;
        wheelRotations(vertical: boolean, rotationUnits: number): DeviceEvent;
        keyPressed(scancode: number): DeviceEvent;
        keyReleased(scancode: number): DeviceEvent;
        unicodePressed(unicode: string): DeviceEvent;
        unicodeReleased(unicode: string): DeviceEvent;
    };
    ConfigParser: { new (config: string): ConfigParser };
}
