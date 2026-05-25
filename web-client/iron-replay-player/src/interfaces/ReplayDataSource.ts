export interface ReplayMetadata {
    durationMs: number;
    totalPdus: number;
    initialWidth?: number;
    initialHeight?: number;
    /**
     * MCS I/O channel ID extracted from the recording's MCS Connect Response.
     * If omitted, the replay engine uses the protocol-common default (1003).
     */
    ioChannelId?: number;
    /**
     * MCS user channel ID extracted from the recording's MCS Attach-User exchange.
     * If omitted, the replay engine uses the protocol-common default (1002).
     */
    userChannelId?: number;
    /**
     * Share ID extracted from the recording's Server Demand Active PDU.
     * If omitted, the replay engine uses the protocol-common default (0x10000).
     */
    shareId?: number;
}

/** Direction of a PDU relative to the RDP session. */
export type PduDirection = 0 | 1;

/** Named constants for PDU direction values. */
export const PduDirection = {
    /** Client → Server. */
    Client: 0 as const,
    /** Server → Client. */
    Server: 1 as const,
} as const;

export interface ReplayPdu {
    timestampMs: number;
    source: PduDirection;
    data: Uint8Array;
}

export interface ReplayDataSource {
    /**
     * Called once when the component initializes.
     * Consumer parses headers / reads metadata, resolves when ready.
     * If this rejects, the component transitions to LoadState 'error'.
     * close() may still be called after a rejected open() (e.g., when
     * a new recording is loaded). Implementations should handle this
     * gracefully (no-op if no resources were allocated).
     */
    open(signal?: AbortSignal): Promise<ReplayMetadata>;

    /**
     * Return PDUs within [fromMs, toMs), sorted by timestampMs ascending.
     * Returns empty array if range contains no PDUs.
     *
     * The signal parameter is an optimization hint for cancellation.
     * The component handles cancellation internally and will discard
     * results from aborted requests. Implementations may:
     * - Pass the signal to native fetch() for network cancellation
     * - Check signal.aborted to short-circuit expensive operations
     * - Ignore the signal entirely (the component handles this correctly)
     *
     * Contract:
     * - Must not return PDUs outside [fromMs, toMs).
     * - The data field must remain valid until the promise settles.
     *   The component consumes data synchronously and does not retain references.
     */
    fetch(fromMs: number, toMs: number, signal?: AbortSignal): Promise<ReplayPdu[]>;

    /**
     * Called on component teardown and when a new recording replaces the
     * current data source. Fire-and-forget.
     *
     * close() may be called after an aborted or rejected open() during
     * reload/re-init. Implementations should handle repeated calls
     * gracefully and no-op if no resources were allocated.
     */
    close(): void;
}

export interface PlayerError {
    message: string;
    phase: 'init' | 'seek' | 'playback';
    cause?: unknown;
}
