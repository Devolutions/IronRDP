# IronRDP Record/Replay Benchmark System — Design

> Status: **design / brainstorming output**. No code changed yet. Mirrors the IronVNC benchmark
> system (`D:\AGENT_KNOWLEDGE_BASE\projects\IronVNC-benchmark-plan.md` / `-results.md`) adapted to
> RDP. Date: 2026-06-03.

## 1. Purpose

A deterministic **record once → replay many** benchmark for the IronRDP **client** decode/render
path, runnable headless (CI-friendly), covering the **web (WASM)** and **.NET/C# (RDM/FFI)**
consumers — not just the native Rust client.

Two things the system must deliver, lose either and the numbers are untrustworthy (inherited from the
IronVNC plan's two "spines"):

1. **A correctness gate**: a canonical framebuffer **CRC32** that every replay must reproduce.
2. **Fresh decode state per measured iteration**: RDP codecs (RemoteFX context, bulk decompressor,
   pointer/palette cache, EGFX/Surface state) are stateful; replaying recorded bytes into a reused
   session from offset 0 desyncs them. Each iteration rebuilds state and replays from the start.

Value: catch **performance regressions** (decode/render got slower) and **correctness regressions**
(pixels changed) across native, web, and .NET, against a shared corpus.

## 2. What we mirror from IronVNC, and what's different

The IronVNC system = one capture format (`IVNCREC1`) consumed by three replay frontends (native
Rust mmap `ReplayTraffic`, WASM `ReplayStream` + `run_web_bench`, .NET `ReplayStream`), plus a
sidecar manifest and a final-framebuffer CRC32. Its measured payoff lived **entirely in the
post-handshake decode→render hot path** (the `get_frame_buffer` full-frame clone was ~80% of
per-event cost; the `copy_region_into` win cut p95 ~93%).

**The one structural difference is the RDP handshake.** VNC's handshake is plaintext and
deterministic, so IronVNC could replay recorded wire bytes straight through the real client
`connect()`. RDP's handshake ends in **CredSSP/NLA** — a live, randomized crypto challenge-response
that cannot be reproduced by replaying recorded server bytes (the replay client generates fresh
nonces the recorded server responses don't answer).

Two ways to get past NLA on replay:

- **Strategy A — skip the handshake.** Replay only the **active-session** server→client stream;
  build the client's `ActiveStage` + `DecodedImage` directly from negotiated parameters stored in the
  manifest. No connect, no TLS, no NLA. This is exactly where all the IronVNC perf value lived.
- **Strategy B — re-synthesize the handshake against an in-process fake server.** Replay runs the
  real client `connect()` against a fake RDP server (reusing `ironrdp-acceptor` + sspi-rs
  `CredSspServer` + an in-memory TLS endpoint with a fixed cert), which performs a *fresh*
  deterministic NLA, then streams the recorded active-session bytes. Exercises the full client path
  including connection + NLA decode.

CredSSP **is** replayable in the Strategy-B sense — proven by sspi-rs (`CredSspServer`,
`ServerMode::Ntlm/Kerberos`, `KdcMock`, `tests/sspi/client_server/credssp.rs`) and by
devolutions-gateway's `credential_injection_kdc.rs` (production in-process fake KDC + CredSSP
acceptor).

## 3. Capture format — `IRDPREC1`

Three files per capture (sidecar pattern, like IVNCREC1 — never patch a header into the byte stream
the background writer owns):

```
<name>.irdprec            stream file (see modes below)
<name>.json               session-level negotiated state (manifest)
<name>.checksum.json      canonical final-framebuffer CRC32 (ground truth)
```

### Stream file modes

- **P-mode (recommended primary): post-parse PDU pairs.** Repeated `{ Action(FastPath|X224) | u32 len
  | payload_bytes }`. Recorded at the client's frame boundary (`ironrdp-client/src/rdp.rs:600`, right
  after `reader.read_pdu()`), replayed by feeding each pair straight into
  `ActiveStage::process(&mut image, action, &payload)`. Bypasses sockets/TLS/MCS framing entirely and
  reproduces a byte-identical framebuffer (confirmed: the decode path is integer-only and
  deterministic given identical PDUs). Portable verbatim to Rust, WASM, and .NET.
- **W-mode (optional): raw decrypted wire bytes.** The full post-TLS server→client byte stream,
  recorded by wrapping the inner stream (`rdp.rs:287`, a `RecordingStream` AsyncRead adapter). Needed
  only when a frontend wants to exercise its own framing layer (`Framed`) or for Strategy B. Re-framed
  on replay via the same `find_size`/`read_pdu` path.

P-mode is the workhorse; W-mode is additive.

### Manifest (configures replay / the fake server)

- protocol/negotiation: desktop size, color depth, **pixel format** (client uses `RgbA32`),
  security protocol used (RDP/SSL/HYBRID).
- negotiated **capability sets** (the full `Vec<CapabilitySet>`).
- connection identifiers: **I/O channel id, user channel id, static virtual channel ids, share id**
  (load-bearing for Strategy B and for slow-path/DVC PDUs — see §6 risk).
- codec config: RemoteFX, EGFX/Graphics-pipeline (AVC444 etc.), QOI/QOIz, NSCodec; DVC/drdynvc setup.
- bookkeeping: frame/PDU count, **encoding histogram** (how many RFX/bitmap/EGFX/… frames), dirty-rect
  area distribution. Honor the manifest as ground truth, never the filename.

### Checksum

CRC32 over the canonical framebuffer = `DecodedImage::data()` (top-down, tightly packed, R,G,B,A, 4
bytes/pixel, stride `width*4`); the sidecar holds just `{ "crc32": "…" }` (dimensions/format live in
the manifest). **Mask or pin the alpha channel** —
most `apply_*` paths force `a=0xFF` but the QOI RGBA path copies source alpha, so canonicalize alpha
before hashing for cross-codec robustness. **Disable the server pointer / software cursor and replay
with zero input events** — cursor compositing mutates the framebuffer and is the single biggest
nondeterminism source.

## 4. Recorder (native Rust tool)

A small headless binary (or a `--record-*` flag set on `ironrdp-client`) that connects to a **real**
RDP server to build the corpus:

- Tap at `rdp.rs:600` (P-mode) and/or `rdp.rs:287` (W-mode), reusing the IronVNC `RecordTraffic`
  shape (background writer thread + unbounded channel; note the IronVNC fix: never `blocking_send`
  inside the tokio runtime).
- Snapshot `ConnectionResult` → manifest at session ignition.
- Compute the final-framebuffer CRC32 at teardown.
- `--exit-after-secs <n>` for bounded headless capture.

**Privacy gate (mandatory, inherited from IronVNC):** RDP captures = screen content. Record **only**
deterministic demo VMs / synthetic content / sanitized sessions. **Never** record real customer
desktops. This gates corpus check-in.

## 5. Replay frontends

| Frontend | Strategy | How | New code |
|---|---|---|---|
| **Web (WASM)** | A | `#[wasm_bindgen] run_web_bench(capture, w, h, canvas, passes, max_frames)` behind a `bench` cargo feature in `ironrdp-web`; builds `ActiveStage`+`DecodedImage` from manifest, feeds P-mode PDUs through the real render loop (`read_pdu`→`active_stage.process`→`extract_partial_image`→`Canvas::draw`), per-stage `performance.now()` timing with overhead calibration + warmup + median-of-passes JSON. Driven by a Playwright harness. | `bench` module (mirror `ironvnc-web/src/bench.rs`); Playwright runner; build `wasm-pack --features bench` (no xtask `?url` rewrite for a standalone page). |
| **.NET / C#** | A | `ReplayStream : System.IO.Stream` (serves recorded bytes, no-ops writes) → `Framed<ReplayStream>` → existing `ActiveStage.Process` loop → CRC `DecodedImage.GetData()`. Fresh `ActiveStage`+`DecodedImage` per iteration, warmup excluded, CRC outside the timed loop, JSON schema from IronVNC Appendix A. | `Devolutions.IronRdp.ReplayBench` (net8) + a `ReplayStream`. **One small FFI addition** likely needed: construct `ActiveStage`/`ConnectionResult` from manifest params (active-session replay has no live `ConnectionResult`). |
| **Native (Rust)** | A (+B) | Recorder host; also self-checks determinism by replaying its own capture and comparing CRC. Mmap `ReplayTraffic` for W-mode if exercising framing. | minimal — reuses recorder. |

Strategy **B (fake-server full handshake)** is a shared Rust component usable by native and .NET
(in-memory duplex or loopback TCP; web is Gateway/WebSocket-bound so it stays on A, matching the
proven IronVNC web bench which bypassed connect). Reuses: `ironrdp-acceptor`
(`accept_begin`/`accept_credssp`/`accept_finalize`, generic over the stream), sspi-rs
`CredSspServer` + `ServerMode::Ntlm` (NTLM needs no KDC/network — simplest), `ironrdp_tls::upgrade`
(works over `tokio::io::DuplexStream`), and a fixed self-signed cert (extract its SPKI for the CredSSP
pubkey binding, like `ironrdp-server/src/helper.rs:45-56`). New code is small (duplex setup, fixed
cert, a ~10-line `CredentialsProxy`). **B is additive coverage of the connect/NLA path — not where
perf wins live.**

## 6. Key findings & constraints (from investigation)

- **Channel-id / share-id pinning (Strategy B only).** `ironrdp-acceptor` hard-codes user=1002,
  io=1003, SVC=auto-increment, share_id=0; none injectable today
  (`crates/ironrdp-acceptor/src/connection.rs:24-25,445-458,632`). If the corpus is recorded from a
  **non-IronRDP** server (Windows/xrdp), the recorded PDUs carry that server's ids and won't line up
  with a fresh fake acceptor → needs a contained acceptor change (constructor params + a name→id map;
  no state-machine surgery). **Strategy A sidesteps this entirely** for fastpath content (fastpath
  frames carry no MCS channel id); slow-path + DVC/EGFX PDUs do embed channel ids, so the
  manifest-built `ActiveStage` must use the recorded ids regardless.
- **Framebuffer extraction** is settled: `DecodedImage::data()` (`crates/ironrdp-session/src/image.rs:176`),
  `width()/height()/stride()`, format `RgbA32`. `ActiveStage::process` mutates the image in place;
  `GraphicsUpdate(rect)` only reports the dirty region.
- **Determinism (post-handshake)** is clean except: (1) server-pointer compositing — disable it;
  (2) `tokio::select!` interleaving — a pure PDU-replay loop removes it; (3) must replay the full PDU
  sequence from the start (stateful decoders) — i.e. fresh-session-per-iteration.
- **.NET seam** needs zero new transport code (transport is C#-owned via `Framed<Stream>`); the only
  likely FFI addition is an `ActiveStage`-from-manifest constructor for handshake-free replay.

## 7. Recommendation (evidence-based)

Build **Strategy A first** as the workhorse across web + .NET (+ native self-check). It is:

- where **all** the IronVNC perf value was found (render/decode hot path),
- the least new code (no fake server, no acceptor changes, no in-WASM TLS/NLA),
- portable verbatim across all three frontends via the P-mode capture.

Add **Strategy B (fake server)** as a **follow-up** for native + .NET, to extend coverage to the
connection + NLA path. You chose B for fidelity, and it is genuinely feasible (sspi-rs + acceptor +
gateway precedent) — this plan keeps it as the second phase rather than the blocker, because the
hot-path numbers (the reason to benchmark at all) don't depend on it.

## 8. Phasing

| Phase | Deliverable | New code |
|---|---|---|
| **0 — Recorder + format** | `IRDPREC1` writer (P-mode + sidecar manifest + checksum) on the native client; record an initial deterministic corpus | recorder tap, manifest/checksum writers |
| **1 — .NET replay (A)** | `Devolutions.IronRdp.ReplayBench` (fresh-session/iter, CRC gate, per-stage timing, JSON) | `ReplayStream`, harness, likely 1 FFI ctor |
| **2 — Web replay (A)** | `ironrdp-web` `bench` feature + `run_web_bench` + Playwright runner | bench module, Playwright harness |
| **3 — Corpus breadth** | RemoteFX, EGFX/AVC444, bitmap, QOI; 1080p + 4K; encoding histograms | captures only |
| **4 — Strategy B (optional)** | shared fake-server (acceptor + sspi NTLM + in-mem TLS); acceptor id-pinning; native + .NET connect-path replay | fake server, acceptor pinning |

## 8b. Phase 0 — as-built (implemented & verified 2026-06-03)

Phase 0 is implemented and verified end-to-end against a live Windows RDP server
(`IT-HELP-DC.ad.it-help.ninja`).

**What shipped:**
- Recorder: `--record-traffic` / `--record-manifest` / `--record-checksum` / `--exit-after-secs` on
  `ironrdp-viewer`, backed by `crates/ironrdp-client/src/record.rs` (`RecordingStream` gated tap +
  `IRDPREC1` manifest + canonical CRC32). Capture is the **active-session** byte stream: the gate
  opens right after `connect_finalize` returns (prepending `framed.peek()`), so no `ironrdp-async`
  change was needed.
- Native replay: `crates/ironrdp-replay-bench` reconstructs a `ConnectionResult` **directly** from the
  manifest (it is a plain `pub struct`, so the `#[non_exhaustive]` connector-state / marked-done path
  was unnecessary) and replays the capture into a fresh `ActiveStage` per iteration.
- The manifest records the negotiated **static channel name→ID map**; replay rebuilds the same
  channel set (drdynvc+DisplayControl+Echo, rdpsnd/rdpdr no-op) with matching IDs so the x224
  processor routes the captured slow-path/DVC traffic. Graphics decode over fast-path.

**Verified:** record (82 PDUs, 1920×1080) → replay reproduces the ground-truth framebuffer CRC32
`3b3f4c25` deterministically across separate invocations (decode ≈250 ms median).

## 8c. Phases 1 & 2 — as-built (implemented & verified 2026-06-03)

**All three frontends reproduce the same capture's ground-truth framebuffer CRC32 `3b3f4c25`.**

- **Phase 1 — .NET (the RDM path):** shared `ironrdp-replay-core` crate (used by native + FFI so they
  build identical `ConnectionResult`s); new FFI `ReplayConnectionBuilder` (`ffi/src/connector/replay.rs`,
  Diplomat bindings regenerated); `Devolutions.IronRdp.ReplayBench` (net8) with a `ReplayStream :
  Stream` + the real `ActiveStage.Process` loop + CRC32 over `DecodedImage.GetData()`. ✅ MATCH
  (decode ≈257 ms median).
- **Phase 2 — Web (WASM):** `bench` feature + `run_web_bench` in `ironrdp-web` (`src/bench.rs`, reuses
  `Canvas` + `extract_partial_image` + `ironrdp-replay-core`), per-stage `performance.now()` timing;
  `bench-harness/` Playwright runner (static server + headless Chromium) loads the wasm, feeds the
  capture, asserts the checksum. ✅ MATCH (read/decode/extract/draw = 0.2/307.5/3.4/356.6 ms).
  Build: `cargo install wasm-bindgen-cli --version <lockfile>` first so `wasm-pack` doesn't try to
  auto-download it; build with `RUSTFLAGS=--cfg getrandom_backend="wasm_js"` and `--features bench`.

**Build-environment note:** this machine has a VS-2026-era cmake-rs generator (`"Visual Studio 18
2026"`) that the installed cmake rejects, breaking `libopus_sys`/`aws-lc-sys` (pulled by the viewer's
audio + rustls). Build the viewer with `CMAKE_GENERATOR=Ninja` (ninja is installed). Incremental Rust
rebuilds reuse the cached C artifacts and need no special env. `ironrdp-replay-bench` does not pull
those C deps.

## 9. Open decisions

1. **Corpus source.** Record against (a) real Windows/xrdp servers (representative content, but
   Strategy B needs acceptor id-pinning) or (b) `ironrdp-server` (ids already match the fake acceptor,
   but synthetic content). Recommendation: real servers for the A-path corpus (ids don't matter for
   fastpath A-replay); revisit for B.
2. **Result schema.** Reuse IronVNC `result.json` (Appendix A of the IronVNC plan) verbatim, trimmed
   to RDP stage names (`readFrame`, `processFrame`, `extractRegion`, `convert`, `present`).
3. **Where the recorder lives.** Flags on `ironrdp-client` vs a dedicated headless `ironrdp-record`
   binary. Recommendation: dedicated binary (decouples record from the GUI client).
```
