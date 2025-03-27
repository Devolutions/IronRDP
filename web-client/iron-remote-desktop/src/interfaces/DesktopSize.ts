export interface DesktopSize {
    width: number;
    height: number;

    construct(width: number, height: number): DesktopSize;
    free(): void;
}
