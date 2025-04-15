import type { ExtensionValue } from './ExtensionValue';

export interface Extension {
    init(ident: string, value: unknown): ExtensionValue;
}
