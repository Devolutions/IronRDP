# Design memo: `Encode`/`Decode`, `WriteBuf`, and Rust ↔ C# buffer ownership in the FFI layer

Static-analysis-only investigation. No benchmarks were run; all current-state claims cite
file and line ranges as of commit `32a8736` on the main development line. Claims that could
not be grounded in the code are flagged in §7.

## 1. Problem restatement

IronRDP encodes PDUs into Rust-owned buffers (`WriteBuf` backed by `Vec<u8>`, or `Vec<u8>`
returned directly by core APIs) and the Diplomat-generated C# bindings then copy those bytes
into C#-owned `byte[]` before they reach the socket; on the receive side, C# accumulates
socket bytes in managed buffers and hands Rust a pinned view to decode from. The question is
whether the encode-side copy (and any decode-side residual copies) can be eliminated without
breaking five invariants: `Encode` object-safety, monomorphization-free decode, `no_std`/
no-alloc infallible cursors, byte-identical wire output, and FFI soundness. The working
hypothesis is that the traits are not the problem — the copies are forced by `WriteBuf`
ownership, the size-unknown-in-advance APIs built on it, and the marshaling choices in
`ffi/`.

## 2. Current-state analysis: the copy/allocation map

The FFI mechanism is [Diplomat](https://github.com/rust-diplomat/diplomat) 0.7
(`ffi/Cargo.toml:15-16`). Two marshaling facts frame everything below:

- **Slice parameters are zero-copy.** Diplomat's C# backend passes `byte[]` arguments by
  pinning them with `fixed` and sending `ptr + len`; Rust receives a real `&[u8]` /
  `&mut [u8]` valid for the duration of the call (e.g. generated
  `ffi/dotnet/Devolutions.IronRdp/Generated/WriteBuf.cs:73-76`,
  `Generated/BytesSlice.cs:62-65`). The pin is released when the call returns.
- **Return values are never slices.** Diplomat cannot return a `&[u8]` directly, so the FFI
  layer wraps returned bytes in opaques: `VecU8` (owned `Vec<u8>`,
  `ffi/src/utils/mod.rs:6-29`) or `BytesSlice<'a>` (borrowed `&'a [u8]`,
  `ffi/src/utils/mod.rs:31-46`), each read out via a `fill(&mut [u8])` copy.

### 2a. Encode, connector path (per `Sequence::step`)

Rust side: `ClientConnector::step` (`ffi/src/connector/mod.rs:155-169`) forwards into
`ironrdp_connector::Sequence::step(&mut self, input: &[u8], output: &mut WriteBuf)`
(`crates/ironrdp-connector/src/lib.rs:324-334`). The state machine encodes into the
Rust-owned `WriteBuf` via `encode_buf` → `unfilled_to(size)`/`advance(written)`
(`crates/ironrdp-core/src/encode.rs:190-200`, `crates/ironrdp-core/src/write_buf.rs:82-85,
161-166`). Allocation is amortized: `clear()` keeps up to 16 KiB of capacity
(`write_buf.rs:4-5, 156-159`). **Copies so far: 0** (encoding is generation, not copying).

C# side: `Connection.SingleSequenceStep` (`ffi/dotnet/Devolutions.IronRdp/src/Connection.cs:119-150`)
allocates `new byte[size]` per step (line 146) and calls `buf.ReadIntoBuf(response)`, which
lands in `WriteBuf::read_into_buf` (`ffi/src/pdu.rs:21-24`) —
`buf.copy_from_slice(&self.0[..buf.len()])`. **1 boundary copy + 1 C# allocation per step.**

There is a worse alternative path: `Framed.Write(WriteBuf)`
(`ffi/dotnet/Devolutions.IronRdp/src/Framed.cs:103-110`) calls `GetFilled`, which clones the
filled region into a fresh `Vec` boxed as `VecU8` (`ffi/src/pdu.rs:26-29`), then `Fill`
copies that into a fresh C# `byte[]`. **2 copies + 1 Rust allocation + 1 C# allocation.**

Compare the native Rust client: `single_sequence_step_write` writes `buf.filled()` straight
to the stream (`crates/ironrdp-async/src/framed.rs:270-289`). **0 copies.** The delta
between native and FFI is exactly the marshaling, not the core APIs.

### 2b. Encode, active-session path (per output frame)

`ActiveStage::process` (`crates/ironrdp-session/src/active_stage.rs:135-197`) allocates a
*fresh* `WriteBuf::new()` per fast-path frame and converts it to a `Vec` with
`into_inner()` for `ActiveStageOutput::ResponseFrame` (lines 142-148) — note there is no
cross-frame amortization here, unlike the connector loop. Related, `process_fastpath_input`
carries an acknowledged copy: `FastPathInput::new(events.to_vec())` is annotated
`// PERF: unnecessary copy` (`active_stage.rs:103-104`), then `encode_vec` allocates an
exact-size `Vec` (`crates/ironrdp-core/src/encode.rs:205-215`).

The FFI exposure of the frame is already zero-copy: `ActiveStageOutput::get_response_frame`
returns `BytesSlice<'_>` borrowing the `Vec` inside the Rust-owned output
(`ffi/src/session/mod.rs:224-231`). The copy happens in C#: allocate `byte[Size]`, `Fill`
(`ffi/dotnet/Devolutions.IronRdp.AvaloniaExample/MainWindow.axaml.cs:506-512`,
`ConnectExample/Program.cs:61-63`). **1 boundary copy + 1 C# allocation per frame.**

Clipboard/DVC/resize helpers (`ffi/src/session/mod.rs:71-137, 144-161`) return `Vec<u8>`
produced by core APIs (`process_svc_processor_messages`, `encode_dvc_messages`,
`active_stage.rs:250-308`) wrapped in `VecU8`; C# reads them via `Utils.VecU8ToByte`
(`Connection.cs:176-185`). **1 boundary copy each** (the `Vec` itself is the core API's
return convention, not an extra copy).

Framebuffer readback: `DecodedImage::get_data` returns a `BytesSlice` over the whole
framebuffer (`ffi/src/session/image.rs:17-20`); the example copies the *entire* image per
render (`MainWindow.axaml.cs:243-247`) even when the dirty region
(`ActiveStageOutput::GraphicsUpdate` rectangle) is small. **1 boundary copy of
`width × height × 4` per graphics update.**

### 2c. Decode path (per inbound frame)

The Rust boundary is already zero-copy: the frame `byte[]` is pinned and passed as `&[u8]`
into `ActiveStage::process` (`ffi/src/session/mod.rs:50-58`) or `Sequence::step`. Decoding
inside Rust borrows from that view (`Decode<'de>` / `ReadCursor<'de>`,
`crates/ironrdp-core/src/decode.rs:178-205`, `cursor.rs:49-144`); data that must outlive the
call is copied into owned Rust structures (inherent and correct — the pin ends at return).

The copies live in the hand-written C# `Framed` layer
(`ffi/dotnet/Devolutions.IronRdp/src/Framed.cs`):

| # | Where | What |
|---|---|---|
| 1 | `Read` (80-87) | socket → 8 KiB temp `byte[]` → `List<byte>.AddRange` (copy) |
| 2..n | `ReadPdu`/`ReadByHint` (26, 122, 147) | `_buffer.ToArray()` copies the **entire accumulated buffer on every size-probe poll**, potentially several times per frame |
| n+1 | `ReadExact` (67) | `Take(size).ToArray()` (copy of the frame) |
| n+2 | `ReadExact` (68) | `Skip(size).ToList()` (copy of the remainder) |

CredSSP round-trips add: `NetworkRequest::get_data` clones the `Vec`
(`ffi/src/credssp/network.rs:88-91`) + `VecU8ToByte` copy (C#); on resume, C# copies
`readBuf` → `actuallyRead` (`Connection.cs:84-88`), the boundary is pinned/zero-copy, then
Rust copies again with `response.to_owned()` (`network.rs:40-43`).

**Copy count summary (per unit):**

| Round-trip | At the Rust↔C# boundary | Rust-side extra | C#-side extra |
|---|---|---|---|
| Connector step (send) | 1 (`ReadIntoBuf`) | 0 | 1 alloc |
| Connector step via `Framed.Write(WriteBuf)` | 1 (`Fill`) | 1 (`GetFilled` clone) | 1 alloc |
| Session response frame (send) | 1 (`BytesSlice.Fill`) | per-frame `WriteBuf`/`Vec` alloc | 1 alloc |
| Inbound frame (receive) | 0 | 0 | 3-5+ copies in `Framed` |
| Framebuffer render | 1 (whole framebuffer) | 0 | 1 alloc + bitmap blit |

## 3. Verdict on the central hypothesis

**Confirmed, with one amendment.** The traits are fine as-is:

- `Encode::encode(&self, &mut WriteCursor<'_>)` writes into *caller-provided contiguous
  memory* — this is precisely the shape needed for encoding directly into a pinned C#
  buffer, with zero trait changes. Object safety is preserved by construction
  (`crates/ironrdp-core/src/encode.rs:157-168`; invariant stated in `ARCHITECTURE.md:66-67`).
- `Decode<'de>` borrowing from `ReadCursor<'de>` already allows decoding directly out of
  pinned C# memory; the pin-scoped lifetime is the only constraint, and it is a marshaling
  constraint, not a trait constraint.

The copies are forced by (a) the FFI marshaling idioms — `VecU8`, `read_into_buf`,
`get_filled` — which exist because Diplomat cannot *return* slices, and (b) `WriteBuf` being
a Rust-owned `Vec` whose contents must therefore either be copied out or exposed as a
transient borrowed view. The amendment: **the single boundary copy is not the dominant
cost**. The C# `Framed` layer performs 3-5+ managed-heap copies per inbound frame (including
a full-buffer copy per size-probe poll) and a fresh allocation per outbound frame. A static
reading strongly suggests fixing `Framed` buys more than eliminating the one boundary copy —
though only a benchmark can rank them (flagged in §7).

`Sequence::step` has no size pre-pass and cannot cheaply grow one: the trait exposes no size
query (`ironrdp-connector/src/lib.rs:324-334`), steps mutate connector state, and CredSSP
steps invoke a generator with network side effects — two-pass "size then encode" is
infeasible there, confirming that the `WriteBuf` (size-unknown) shape is necessary for case B.

**On `size()` exactness (case A precondition):** exactness is an *enforced dynamic
invariant*, not a static one. `encode_buf` and `encode_vec` both
`debug_assert_eq!(written, pdu_size)` (`encode.rs:197, 213`), and the async write path
asserts `buf.filled_len() == response_len` (`ironrdp-async/src/framed.rs:279`). Critically,
exactness is *already load-bearing for wire correctness*: `encode_vec` returns the full
`vec![0; pdu.size()]` regardless of how many bytes were actually written
(`encode.rs:205-215`), so an overestimating `size()` would emit trailing zero bytes on the
wire in release builds today. Pre-sizing a C# buffer from `size()` therefore adds no new
requirement — but any FFI consuming it must still treat the returned `written` count as
authoritative. I did not audit every `size()` implementation across `ironrdp-pdu` (§7).

## 4. Per-case options

| Case | Option | Copies eliminated | Invariants touched | Soundness hazards | FFI-ergonomics cost |
|---|---|---|---|---|---|
| A (size known) | **A1**: status quo — core returns `Vec` → `VecU8` → `Fill` | — (baseline: 1 copy + 1 Rust alloc) | none | none | none |
| A | **A2**: C# allocates `byte[pdu.size()]`, pins, Rust runs `ironrdp_core::encode(pdu, dst)` (`encode.rs:171-178`) over it | 1 copy + 1 Rust alloc → **0** | none — pure FFI addition; traits untouched | pin is call-scoped (safe); must return `written` and require impls' `ensure_size!` guards (clean error, not panic, when undersized) | small: needs a `size()`-exposing FFI method per PDU opaque; today **no FFI surface encodes a bare `Encode` PDU**, so this is future-proofing |
| B (size unknown) | **B-i**: Rust retains `WriteBuf`; expose `filled()` as `BytesSlice<'_>` view; C# writes it to the socket via `ReadOnlySpan<byte>`/`MemoryManager` | boundary copy → **0** | none in core; parallel FFI method | view invalidated by next `step`/`clear`/`Dispose`; Diplomat C# does **not** enforce the borrow (§5); async `WriteAsync` extends the window across `await` | moderate: unsafe span construction in C#, strict "no step while view live" contract |
| B | **B-ii**: keep one copy — `read_into_buf` into a pooled (`ArrayPool<byte>`) C# buffer sized from `Written` | allocs → 0; keeps exactly **1** copy | none | none new (fix the `buf.len() > filled` panic first, §5) | trivial |
| B | **B-iii**: C#-provided grow callback backs `WriteBuf` storage | 1 copy | **core**: `WriteBuf` storage is hardcoded `Vec<u8>` (`write_buf.rs:19-22`); would need a storage abstraction or custom allocator | callback reentrancy, C# exceptions unwinding into Rust, GC pinning of indefinite duration, panic across `extern "C"` | **disqualified**: high soundness cost, touches core, saves nothing over B-i |
| C (decode) | **C1**: status quo — pinned input view, owned Rust outputs, field extraction copies | — (boundary already 0-copy) | none | none | none |
| C | **C2**: fix the C# `Framed` layer (ring/pooled buffer, span-based size probes via raw `AsFFI` pointers, no `List<byte>`) | 3-5+ managed copies per frame → ~1 | none — zero Rust changes | `Peek()`-style span discipline (comment at `Framed.cs:46-54` already states it) | small-moderate, pure C# work |
| C | **C3**: return borrowed decode outputs (`'de` tied to C# input) across FFI | extraction copies | none in traits | **not enforceable**: pin ends at call return, and generated C# wrappers hold no parent reference — use-after-free by construction unless outputs are consumed within the call | disqualified as a general mechanism; viable only for call-scoped callbacks |

## 5. Soundness

**Pinning.** Diplomat's `fixed`-based marshaling pins C# arrays only for the duration of the
native call. Building a `WriteCursor`/`ReadCursor` over the pinned pointer inside the call
(options A2, C1) is sound; retaining the pointer in any Rust structure past return is UB
(the GC may move or collect the array). Nothing in the current bindings retains such a
pointer — every `&[u8]`/`&mut [u8]` parameter is consumed or copied within the call
(`ffi/src/connector/mod.rs:155-161`, `ffi/src/credssp/network.rs:40-43`). Any new API must
keep that property; long-lived C# buffers handed to Rust would instead require
`GCHandle.Alloc(..., Pinned)` or native memory on the C# side, which is a heavier contract
than any option here needs.

**Borrowed-view lifetimes (B-i, and the existing `BytesSlice`).** The Rust bridge is
lifetime-correct (`BytesSlice<'a>`, `PduHint<'a>`, `DynState<'a>` are declared with
lifetimes Diplomat validates), but the generated C# holds only a raw pointer — no reference
to the parent object (`Generated/BytesSlice.cs:14-38`, `Generated/ActiveStageOutput.cs:116-132`).
If the parent `ActiveStageOutput` is disposed or GC-finalized, or the `WriteBuf` is
mutated/cleared, while a `BytesSlice` is alive, `Fill` reads freed or stale memory. **This
hazard exists in the shipped bindings today**; current example code survives by scope
discipline (view created, filled, and dropped within one block). B-i widens the window and
must ship with an explicit contract: the view is valid only until the next mutating call on
its source, and must not be held across `await`. A debug-only generation counter on the FFI
`WriteBuf` wrapper (view captures the counter; `fill` asserts it) would make violations
deterministic instead of heap-dependent, without touching core.

**Aliasing and reentrancy.** Diplomat methods on opaques take `&mut self` reconstructed from
a raw pointer. C# enforces no exclusivity: two threads calling `step` on one connector, or
`Fill` racing `Clear`, is a data race → UB. The de facto contract is "one logical thread per
object" (the C# `Framed` even carries its own write mutex, `Framed.cs:9`, hinting at
multi-threaded callers). Any zero-copy view API sharpens this from "race on internals" to
"race on a slice being read while reallocated", so the single-threaded-per-object contract
should be documented as a hard rule, not folklore.

**Who frees.** Clean today and unchanged by the recommendations: Rust-boxed opaques are
freed by C# `Dispose`/finalizer calling the generated `Destroy` (Rust `Box` drop); C#
arrays are GC-managed; neither side ever frees the other's allocation. `BytesSlice`'s
destroy frees only the box holding the reference, never the pointee. B-iii is the only
option that would have blurred this line — another reason it is disqualified.

**Panics across the boundary.** `WriteBuf::read_into_buf` panics if C# passes a buffer
longer than the filled region (`ffi/src/pdu.rs:22` slices `self.0[..buf.len()]`;
`Index<RangeTo>` on the filled region, `write_buf.rs:227-233`), and `VecU8::fill`/
`BytesSlice::fill` panic on *longer*-than-content buffers (`copy_from_slice` requires equal
lengths after the too-small check, `utils/mod.rs:18-24, 39-45`). A panic unwinding out of an
`extern "C"` function aborts the process (Rust ≥ 1.81), unless Diplomat wraps calls in
`catch_unwind` — I found no evidence it does in 0.7 (§7). These should return errors instead;
B-ii's "write up to `min(len, filled)` and return the count" variant fixes this for free.

## 6. Recommendations and migration path

**Case A — adopt the pattern, no urgency.** There is no FFI surface today that encodes a
bare `Encode` PDU, so nothing to migrate. When one appears, the shape is: expose `size()`,
let C# allocate and pin, run `ironrdp_core::encode` over the pinned slice, return `written`.
Zero trait changes, zero copies. Document (doc-comment only, no behavior change) that
`Encode::size()` is an exact-size contract — §3 shows it already is one in practice.

**Case B — B-ii now, B-i as an opt-in fast path.** The pragmatic sweet spot is keeping
exactly one boundary copy with zero allocations: pool the destination buffers in C#
(`ArrayPool<byte>`), size them from `Written`, and fix `read_into_buf` to be non-panicking
and count-returning (parallel method, e.g. `read_into(&mut [u8]) -> usize`; keep the old one
for ABI stability). For the throughput-critical active-session send path, add
`WriteBuf::get_filled_view() -> BytesSlice<'_>` (mirroring `DecodedImage::get_data`,
`ffi/src/session/image.rs:17-20`) so advanced C# callers can `Stream.Write(ReadOnlySpan<byte>)`
directly from Rust memory — synchronous write only, view dead after the next `step`/`clear`,
single-threaded contract, ideally with the debug generation-counter guard from §5. All of
this is additive FFI-layer work; `WriteBuf`, `Sequence`, and the traits are untouched, so
invariants 1-4 hold trivially.

**Case C — fix the C# `Framed` layer first; keep owned decode outputs.** Replace
`List<byte>` + `ToArray`-per-poll with a pooled growable buffer and span-based size probes
(the raw `AsFFI` pointers make a span-accepting probe possible without regenerating
bindings). This eliminates the largest copy count in the whole round-trip with zero Rust
changes and zero soundness surface. Do *not* attempt borrowed decode outputs across the
boundary: the binding mechanism can express the lifetime but cannot enforce it (§5), so the
current design — decode inside Rust from the pinned input, keep outputs owned in Rust
opaques, extract fields on demand (one copy per field the app actually persists) — is the
correct resting point. The framebuffer path deserves one targeted addition: a
region-bounded read (`get_data_region(rect, &mut [u8])`) so renders copy the dirty rectangle
instead of the whole framebuffer per `GraphicsUpdate`.

**Sequencing.** (1) C# `Framed` rewrite + pooled buffers (pure C#, biggest static win);
(2) non-panicking `read_into` + `get_filled_view` + `get_data_region` (additive Rust FFI);
(3) `size()` doc-contract + case-A encode-in-place pattern when a sized-PDU FFI surface
first appears; (4) independently, the `// PERF: unnecessary copy` in
`process_fastpath_input` (`active_stage.rs:103-104`) and the per-frame `WriteBuf::new()` in
`ActiveStage::process` (`active_stage.rs:142-148`) are core-side allocation reductions that
benefit all consumers, not just FFI — worth separate, small PRs. Everything above is a
parallel API or an internal change; nothing modifies existing signatures, so invariants 1-5
survive by construction.

## 7. Open questions / not determinable statically

- **Actual bottleneck ranking.** Static reading says the C# `Framed` copies dominate the
  single boundary copy, but only profiling a real session (large fast-path graphics frames
  vs. many small PDUs) can rank them.
- **`size()` exactness audit.** Enforcement is `debug_assert`-based; I verified the
  enforcement points, not the ~hundreds of `size()` implementations across `ironrdp-pdu`
  and channel crates. A release-mode overestimate would already corrupt `encode_vec` output
  (§3), which argues no such bug ships — but that is inference, not proof. A one-off
  fuzz/property test asserting `encode_vec(pdu).len() == cursor.pos()` per PDU would settle it.
- **Diplomat panic behavior.** I found no `catch_unwind` in the generated C# or the bridge
  macro surface, but the macro-expanded Rust `extern "C"` shims were not inspected
  (generated at compile time). Whether a Rust panic aborts or is caught should be verified
  before relying on panics as "safe" failure modes (file to check: Diplomat's generated
  bridge expansion, e.g. via `cargo expand -p ffi`).
- **Async writes over borrowed views (B-i).** Wrapping Rust memory in a
  `MemoryManager<byte>` to use `WriteAsync(ReadOnlyMemory<byte>)` keeps the view alive
  across `await`, exactly the window §5 warns about. Whether the .NET socket stack completes
  synchronously often enough to make sync `Write(ReadOnlySpan<byte>)` acceptable is an
  empirical question.
- **Multi-threaded C# consumers.** The write mutex in `Framed.cs:9` suggests real callers
  touch the session from multiple threads; the actual threading model of downstream
  consumers (e.g. Devolutions products embedding this NuGet) determines how much weight the
  single-threaded-per-object contract can bear and whether the debug generation counter
  should instead be a release-mode check.
