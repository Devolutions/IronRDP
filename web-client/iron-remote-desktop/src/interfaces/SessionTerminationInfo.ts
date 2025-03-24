export interface SessionTerminationInfo {
    free(): void;
    reason(): string;
}
