import type { DesktopSize as IDesktopSize } from './../../../iron-remote-desktop/src/interfaces/DesktopSize';
export class DesktopSize implements IDesktopSize {
    constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
    }

    init(width: number, height: number): DesktopSize {
        return new DesktopSize(width, height);
    }

    width: number;
    height: number;
}
