# ironrdp-web-replay

WebAssembly replay engine for pre-recorded RDP sessions. Decodes FastPath
PDUs into an RGBA framebuffer and blits the result onto an HTML `<canvas>`
with cursor compositing.

Reuses IronRDP's existing `ironrdp-session` FastPath processor and
`ironrdp-graphics` pipeline.

## WASM API

The crate exposes these types to JavaScript via `wasm-bindgen`:

- **`new Replay(canvas)`** — Bind a `<canvas>` element, allocate the RGBA
  framebuffer (default 1920×1080), and initialize the 2D rendering context.

- **`pushPdu(timestampMs, source, data)`** — Enqueue a raw PDU with its
  recording-time offset (ms from session start) and direction
  (client→server / server→client) into an internal FIFO buffer.

- **`renderTill(elapsedMs)`** — Drain all buffered PDUs up to `elapsedMs`,
  decode each through the FastPath processor, and blit dirty frames to the
  canvas with cursor compositing. Returns a `RenderResult` with
  `pdus_processed`, `resolution_changed`, and `session_ended` flags.

- **`setUpdateCanvas(bool)`** — When `false`, `renderTill()` still decodes
  PDUs and tracks pointer state but skips canvas blits. Used during forward
  seeks to fast-forward without painting intermediate frames.

- **`forceRedraw()`** — Unconditionally blit the current framebuffer to
  canvas. Called after a forward seek completes to paint the final frame.

- **`reset()`** — Clear the framebuffer, PDU buffer, processor, and cursor
  state. Used for backward seeks (replay from the beginning).

## Build

```sh
# From the repo root
cargo xtask web build-replay -v
```

This runs `wasm-pack build`, patches the generated JS for Vite
compatibility, and builds the TypeScript adapter library
(`web-client/iron-replay-player-wasm`).

## Tests

The crate has `test = false` because it targets `wasm32-unknown-unknown`
(the native test harness cannot run on WASM).
Tests live in `ironrdp-testsuite-extra`, which is the home for extra-tier integration tests.

```sh
# Run all web-replay tests
cargo test -p ironrdp-testsuite-extra -- web_replay

# Run a specific test
cargo test -p ironrdp-testsuite-extra -- web_replay::pdu_buffer_clear_empties_buffer
```

Covered so far:

- `PduBuffer` — FIFO ordering, push/peek/clear lifecycle, source direction
- `ReplayProcessor::process_pdu` — error cases (empty, truncated, garbage), client FastPath mouse extraction, seek suppression
- `ReplayProcessor::process_till` — timestamp-gated consumption, mouse position tracking, seek suppression, malformed-PDU error collection
- X224 Server Demand Active — resolution change detection and image reallocation
- Seek contract — `update_canvas` toggle, pointer state defaults

## Known limitations

- **Hardcoded channel IDs** — `io_channel_id` (1003), `user_channel_id`
  (1002), and `share_id` (0x0001_0000) are common defaults. A fully correct
  implementation should extract these from the recorded MCS Connect Response
  and Server Demand Active exchange.

- **Client input limited to mouse** — Only `MouseEvent` and `MouseEventEx`
  are decoded from client→server FastPath PDUs. Keyboard events are skipped.

- **No dynamic virtual channel replay** — Clipboard, audio, and drive
  redirection data is not processed.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
