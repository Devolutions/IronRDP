export interface DeviceEvent {
    free(): void;
    new_mouse_button_pressed(button: number): DeviceEvent;
    new_mouse_button_released(button: number): DeviceEvent;
    new_mouse_move(x: number, y: number): DeviceEvent;
    new_wheel_rotations(vertical: boolean, rotation_units: number): DeviceEvent;
    new_key_pressed(scancode: number): DeviceEvent;
    new_key_released(scancode: number): DeviceEvent;
    new_unicode_pressed(unicode: string): DeviceEvent;
    new_unicode_released(unicode: string): DeviceEvent;
}
