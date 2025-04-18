import { DisplayControl, KdcProxyUrl, Pcb } from '../../../../crates/ironrdp-web/pkg';

export function preConnectionBlob(value: string): Pcb {
    return Pcb.new(value);
}

export function displayControl(value: boolean): DisplayControl {
    return DisplayControl.new(value);
}

export function kdcProxyUrl(value: string): KdcProxyUrl {
    return KdcProxyUrl.new(value);
}
