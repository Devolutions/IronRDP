export interface DesktopSize {
    width: number;
    height: number;

    init(width: number, height: number): DesktopSize;
}
