import wasm_init, {
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    SessionBuilder,
    ClipboardData,
    Extension,
    RdpFile,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';

export async function init(log_level: string) {
    await wasm_init();
    setup(log_level);
}

export { RdpFile };

export const Backend = {
    DesktopSize: DesktopSize,
    InputTransaction: InputTransaction,
    SessionBuilder: SessionBuilder,
    ClipboardData: ClipboardData,
    DeviceEvent: DeviceEvent,
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
