import { Guid } from 'guid-typescript';

export class Session {
    id: Guid;
    sessionId!: number;
    name?: string;
    active!: boolean;
    desktopSize!: { width: number; height: number };

    constructor(name?: string) {
        this.id = Guid.create();
        this.name = name;
        this.active = false;
    }
}
