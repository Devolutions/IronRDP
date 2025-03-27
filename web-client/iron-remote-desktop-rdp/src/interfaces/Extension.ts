type ExtensionValue = { KdcProxyUrl: string } | { Pcb: string } | { DisplayControl: boolean };
export class Extension {
    static construct(ident: string, value: unknown): ExtensionValue {
        switch (ident) {
            case 'kdc_proxy_url':
                if (typeof value === 'string') {
                    const ext: ExtensionValue = { KdcProxyUrl: value };
                    return ext;
                } else {
                    throw new Error('KdcProxyUrl must be a string');
                }
            case 'pcb':
                if (typeof value === 'string') {
                    const ext: ExtensionValue = { Pcb: value };
                    return ext;
                } else {
                    throw new Error('Pcb must be a string');
                }
            case 'display_control':
                if (typeof value === 'boolean') {
                    const ext: ExtensionValue = { DisplayControl: value };
                    return ext;
                } else {
                    throw new Error('DisplayControl must be a boolean');
                }
            default:
                throw new Error(`Invalid extension type: ${ident}`);
        }
    }
}
