export interface Extension {
    construct(ident: string, value: unknown): Extension;
}
