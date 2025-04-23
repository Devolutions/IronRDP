import wasm_init, {
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    SessionBuilder,
    ClipboardData,
    Extension,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';

export async function init(log_level: string) {
    await wasm_init();
    setup(log_level);
}

export const Backend = {
    createDesktopSize: DesktopSize.init,
    createMouseButtonPressed: DeviceEvent.mouse_button_pressed,
    createMouseButtonReleased: DeviceEvent.mouse_button_released,
    createMouseMove: DeviceEvent.mouse_move,
    createWheelRotations: DeviceEvent.wheel_rotations,
    createKeyPressed: DeviceEvent.key_pressed,
    createKeyReleased: DeviceEvent.key_released,
    createUnicodePressed: DeviceEvent.unicode_pressed,
    createUnicodeReleased: DeviceEvent.unicode_released,
    createInputTransaction: InputTransaction.init,
    createSessionBuilder: SessionBuilder.init,
    createClipboardData: ClipboardData.init,
};

export function preConnectionBlob(pcb: string): Extension {
    return new Extension('pcb', pcb);
}

export function displayControl(enable: boolean): Extension {
    return new Extension('display_control', enable);
}

export function kdcProxyUrl(url: string): Extension {
    return new Extension('kdc_proxy_url', url);
}
