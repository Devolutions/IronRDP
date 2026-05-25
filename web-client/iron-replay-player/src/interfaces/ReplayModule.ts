/**
 * Shape of the RenderResult struct returned by renderTill().
 * Matches crates/ironrdp-web-replay/src/replay.rs RenderResult fields.
 */
export interface RenderResult {
    current_time_ms: number;
    pdus_processed: number;
    resolution_changed: boolean;
    session_ended: boolean;
}

/**
 * Configuration for replay processor initialization.
 * Matches the wasm-bindgen-generated ReplayConfig class shape.
 * All fields are optional -- unset fields use protocol-common defaults.
 *
 * Field names use snake_case to match the Rust struct fields as generated
 * by wasm-bindgen (no js_name override).
 */
export interface ReplayConfigInstance {
    io_channel_id?: number;
    user_channel_id?: number;
    share_id?: number;
}

/**
 * Minimal interface for a single WASM replay engine instance.
 *
 * `pushPdu` takes `source` as `number` (0 = Client, 1 = Server) rather than
 * the WASM `PduSource` enum for compatibility with `ReplayDataSource`.
 */
export interface WasmReplayInstance {
    free(): void;
    /**
     * Initialize the replay processor. Must be called once after construction,
     * before renderTill/reset/setUpdateCanvas.
     * @param config - Optional MCS channel/share ID overrides.
     * @throws If already initialized.
     */
    init(config?: ReplayConfigInstance): void;
    /** source: 0 = Client, 1 = Server (matches PduSource enum wire values) */
    pushPdu(timestamp_ms: number, source: number, data: Uint8Array): void;
    /**
     * Process PDUs up to target_ms and blit to canvas.
     * @throws If init() has not been called.
     */
    renderTill(target_ms: number): RenderResult;
    /**
     * Reset the replay engine to initial state. Auto-rebuilds processor from stored config.
     * @throws If init() has not been called.
     */
    reset(): void;
    /**
     * Enable/disable canvas updates. When false, PDUs are processed but not drawn (seek mode).
     * @throws If init() has not been called.
     */
    setUpdateCanvas(update: boolean): void;
    /** Unconditionally blit the current framebuffer to the canvas. Used after seek. */
    forceRedraw(): void;
}

/**
 * Dependency-injection interface for the WASM replay backend.
 * iron-replay-player depends on this interface only -- not on any specific WASM import.
 * iron-replay-player-wasm's ReplayBackend satisfies this interface.
 */
export interface ReplayModule {
    /** Construct a new replay engine bound to a canvas element. */
    Replay: { new (canvas: HTMLCanvasElement): WasmReplayInstance };
    /**
     * PduSource enum: at minimum exposes Client and Server as number values.
     * wasm-bindgen generates this as a plain object with numeric properties.
     */
    PduSource: { readonly Client: number; readonly Server: number };
    /**
     * Optional config constructor for replay initialization.
     * Instances must satisfy ReplayConfigInstance.
     * Non-WASM backends may omit this.
     */
    ReplayConfig?: { new (): ReplayConfigInstance };
}
