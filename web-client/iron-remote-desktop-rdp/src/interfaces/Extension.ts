type ExtensionValue = { Pcb: string } | { KdcProxyUrl: string } | { DisplayControl: boolean };

export class Extension {
    static init(ident: string, value: unknown): ExtensionValue {
        switch (ident) {
            case 'Pcb':
                if (typeof value === 'string') {
                    return { Pcb: value };
                } else {
                    throw new Error('Pcb must be a string');
                }
            case 'KdcProxyUrl':
                if (typeof value === 'string') {
                    return { KdcProxyUrl: value };
                } else {
                    throw new Error('KdcProxyUrl must be a string');
                }
            case 'DisplayControl':
                if (typeof value === 'boolean') {
                    return { DisplayControl: value };
                } else {
                    throw new Error('DisplayControl must be a boolean');
                }
            default:
                throw new Error(`Invalid extension type: ${ident}`);
        }
    }
}
