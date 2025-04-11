export interface DeviceEvent {
    mouse_button_pressed(button: number): DeviceEvent;
    mouse_button_released(button: number): DeviceEvent;
    mouse_move(x: number, y: number): DeviceEvent;
    wheel_rotations(vertical: boolean, rotation_units: number): DeviceEvent;
    key_pressed(scancode: number): DeviceEvent;
    key_released(scancode: number): DeviceEvent;
    unicode_pressed(unicode: string): DeviceEvent;
    unicode_released(unicode: string): DeviceEvent;
}
