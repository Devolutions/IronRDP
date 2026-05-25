import type { LoadState } from './LoadState.js';
import type { PlayerError, ReplayDataSource } from './ReplayDataSource.js';

/** Public API object dispatched via the 'ready' CustomEvent on <iron-replay-player>. */
export interface PlayerApi {
    /** Load a new recording from a data source. Resets all playback state. */
    load(dataSource: ReplayDataSource): Promise<void>;
    /** Start playback. No-op if already playing. */
    play(): void;
    /** Pause playback. No-op if already paused. */
    pause(): void;
    /** Toggle play/pause. */
    togglePlayback(): void;
    /** Seek to an absolute position in milliseconds. */
    seek(positionMs: number): Promise<void>;
    /** Set playback speed multiplier (e.g. 1, 1.5, 2, 3). */
    setSpeed(speed: number): void;
    /** Playback speed multiplier (1.0 = normal speed). */
    getSpeed(): number;
    /** Current playback position in milliseconds (affected by speed and seeking). */
    getElapsedMs(): number;
    /** Total duration in milliseconds (0 if not loaded). */
    getDurationMs(): number;
    isPaused(): boolean;
    /** Current load state — use to check for errors after the player is ready. */
    getLoadState(): LoadState;
    /** Current fetch error, if any. Null when no error is active or after clearError(). */
    getPlayerError(): PlayerError | null;
    /** Reset the active fetch error. Consumer is responsible for retrying the failed operation. */
    clearError(): void;
    /** Seek to position 0, preserving play/pause state. */
    reset(): Promise<void>;
}
