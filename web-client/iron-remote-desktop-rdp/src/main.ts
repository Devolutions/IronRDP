import init, {
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    IronError,
    Session,
    SessionBuilder,
    SessionTerminationInfo,
    ClipboardData,
    ClipboardItem,
    Extension,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';

export default {
    init,
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    IronError,
    SessionBuilder,
    ClipboardData,
    ClipboardItem,
    Session,
    SessionTerminationInfo,
    Extension,
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
