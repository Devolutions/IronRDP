# iron-svelte-replay-client

Demo application showing how to integrate the `<iron-replay-player>` web component
with real data sources. Use this as a reference for wiring the WASM backend, web
component, and your own `ReplayDataSource` implementation together.

Two data source modes are included:

- **HTTP Byte-Range** (`/http`) — Stream a recording from a URL using HTTP Range requests.
  Only fetches the bytes needed for the current playback window.
- **Local File** (`/file`) — Drag-and-drop or select a `.bin` recording file from disk.
  The entire file is read into memory once.

Both modes use a custom binary recording format described at the end of this document.

## Library modules

### `fetchRecording.ts`

HTTP fetch layer for byte-range requests against a recording URL. Handles Range header
construction, `206 Partial Content` validation, and header/index-table parsing.

Supports a `FetchOptions` parameter that accepts static headers, a sync callback, or an
async callback — enabling transparent token rotation without reloading the player.
Throws `FetchHttpError` with the HTTP status code on non-2xx responses.

All functions accept an `AbortSignal` for cancellation on seek, speed change, or teardown.

### `HttpRangeDataSource.ts`

Implements `ReplayDataSource` for HTTP-hosted recordings. On `open()`, fetches the header
and full index table in two Range requests. On `fetch(fromMs, toMs)`, uses binary search
to find the relevant PDU range, then issues a single Range request for the contiguous byte
span and slices the response into individual PDUs.

### `LocalFileDataSource.ts`

Implements `ReplayDataSource` for local `File`, `Blob`, or `ArrayBuffer` inputs. On `open()`,
reads the entire file into memory and parses the header and index table in-place with
validation (version check, PDU count cap, boundary checks). On `fetch(fromMs, toMs)`,
returns zero-copy `Uint8Array` subviews into the original buffer.

### `ReplayDataSource.types.ts`

Mirrors the `ReplayDataSource` interface from `iron-replay-player` using structural typing,
avoiding a direct package dependency between the demo app and the web component.

## Development

### Prerequisites

- Rust toolchain (see `rust-toolchain.toml` at the repo root)
- `wasm-pack` — installed automatically by `cargo xtask web install-replay`
- Node.js + npm

### Quick start

```sh
# From the repo root — install all tools and npm deps
cargo xtask web install-replay -v

# Build WASM + adapter + component
cargo xtask web build-replay -v

# Start the dev server (skips WASM rebuild, uses existing build)
cd web-client/iron-svelte-replay-client
npm run dev-no-wasm
```

Or the all-in-one command that builds everything then starts the server:

```sh
cd web-client/iron-svelte-replay-client
npm run dev-all
```

The server runs at `http://localhost:5173`.

### Replay server

The HTTP Byte-Range demo (`/http`) requires a server that supports
[HTTP Range requests](https://developer.mozilla.org/en-US/docs/Web/HTTP/Range_requests) —
it must return `206 Partial Content` with a `Content-Range` header for `Range` requests.
A `200 OK` response to a Range request will cause the player to reject the response for
non-zero byte offsets.

A sample recording and a zero-dependency server script are included:

```sh
npm run replay-server
```

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `8000` | Port to listen on |
| `--file` | `samples/sample.bin` | Recording file to serve |

Custom usage:

```sh
node scripts/replay-server.mjs --port 9000 --file /path/to/recording.bin
```

Any static file server that supports Range requests will also work (e.g. nginx, caddy,
S3 presigned URLs).

### npm scripts

| Script | Description |
|--------|-------------|
| `dev-all` | Full build (WASM + adapter + component) then start dev server |
| `dev-no-wasm` | Rebuild adapter + component only, then start dev server |
| `dev` | Start dev server (assumes everything is already built) |
| `build` | Full production build |
| `build-no-wasm` | Production build, skip WASM recompile |
| `check` | Run `svelte-check` for TypeScript diagnostics |
| `lint` | Prettier + ESLint |
| `replay-server` | Serve a sample recording with HTTP Range support (port 8000) |

## Recording file format

The recording is a self-contained binary file with three consecutive sections.
All multi-byte integers are **big-endian**.

### Header (20 bytes)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 4 bytes | `version` | Header format version (`uint32`, must be `1`) |
| 4 | 8 bytes | `totalPdus` | Total number of PDUs in the recording (`uint64`) |
| 12 | 8 bytes | `duration` | Total session duration in milliseconds (`uint64`). May be `0` — the player falls back to the last index entry's timestamp. |

### Index table (17 bytes × `totalPdus`)

Immediately follows the header. Each row describes one PDU, enabling random-access
byte-range fetching without scanning the full file.

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 4 bytes | `timeOffset` | Milliseconds since session start (`uint32`) |
| 4 | 4 bytes | `pduLength` | Length of this PDU in bytes (`uint32`) |
| 8 | 8 bytes | `byteOffset` | Absolute byte offset of this PDU in the file (`uint64`) |
| 16 | 1 byte | `direction` | `0x00` = client→server, `0x01` = server→client |

### PDU data (variable)

Raw RDP PDU bytes, concatenated in index order. Each PDU's position and length
are described by its index table entry. The PDUs are the same wire-format bytes
that `ironrdp-web-replay` expects — FastPath, X224/MCS, etc.

This library is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
