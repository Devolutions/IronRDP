import type { DesktopSize as IDesktopSize } from './../../../iron-remote-desktop/src/interfaces/DesktopSize';
export class DesktopSize implements IDesktopSize {
    constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
    }
    // eslint-disable-next-line @typescript-eslint/no-empty-function
    free(): void {}

    construct(width: number, height: number): DesktopSize {
        return new DesktopSize(width, height);
    }

    width: number;
    height: number;
}
