export interface SessionTerminationInfo {
    reason(): string;
    free(): void;
}
