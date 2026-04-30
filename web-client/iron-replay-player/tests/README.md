# Test Suite

Tests for `iron-replay-player`, the RDP session replay web component.

## Running

```bash
# Unit tests (jsdom, fake timers)
npm test
npm run test:watch

# Browser tests (Playwright + Chromium)
npx playwright install chromium   # first-time setup
npm run test:browser
npm run test:browser:headed       # visible browser for debugging
```

## Test tiers

| Tier | Environment | Config | What it tests |
|------|-------------|--------|---------------|
| Unit | jsdom + fake timers | `vitest.config.ts` | Store state machine, pure functions |
| Browser | Chromium via Playwright | `vitest.browser.config.ts` | UI interactions, DOM rendering, pointer events |

Unit tests mock the WASM engine and data source entirely (no real decoding or network calls). Browser tests render the full Svelte component in a real browser with the same mocks.

---

## Unit tests

### Fake timers

All store tests run under `vi.useFakeTimers()`. Time does not advance unless you tell it to:

- `vi.advanceTimersToNextFrame()`: advance one rAF tick (16ms). This is how the store's render loop fires.
- `vi.advanceTimersByTimeAsync(0)`: flush the microtask queue without advancing time. Use this after resolving a fetch to let the store's `await` continuations run.

### Setup helpers

Every store test starts from one of these entry points:

| Helper | What it does | State after |
|--------|-------------|-------------|
| `initStore()` | Opens the data source + wires in the WASM mock | `loadState: ready`, `paused: true`, `elapsed: 0` |
| `startPlayback()` | `initStore()` → `play()` → resolve initial fetch | `paused: false`, rAF loop scheduled, `fetchedUntilMs = 15_000` |
| `seekTo(ms)` | Seek to a position, resolving all chunks with empty PDUs | `elapsed: ms`, `fetchedUntilMs: ms` |

`startPlayback()` resolves the initial fetch with `makePdus(0, 15_000, 100)`, producing 150 PDUs at 100ms intervals from 0 to 14_900. `fetchedUntilMs` advances to 15_000 (the requested range end, not the last PDU timestamp). The rAF loop is scheduled but no frame has fired yet, so `elapsed` is still 0.

### Mock data source (deferred pattern)

`createMockDataSource()` returns a data source where every `fetch()` call returns a **pending promise**. The test controls when it resolves:

```ts
store.play();                              // triggers fetchAndPush → fetch() called → promise pending
ds.resolveFetch(makePdus(0, 15_000, 100)); // test resolves it with PDU data
await vi.advanceTimersByTimeAsync(0);       // flush so the store processes the result
```

Key methods:
- `ds.resolveFetch(pdus)`: resolve the oldest pending fetch
- `ds.rejectFetch(error)`: reject the oldest pending fetch
- `ds.pendingCount`: how many fetches are waiting to be resolved

This gives full control over async timing, which is essential for testing race conditions and abort behavior.

### Mock WASM replay

`createMockWasmReplay()` returns a WASM mock where all methods are `vi.fn()` spies. `renderTill()` returns a default `RenderResult` (no errors, no session end). You can change its behavior:

- `mock.setSessionEnded()`: make `renderTill` return `session_ended: true`
- `mock.setRenderError(err)`: make `renderTill` throw

### Buffer config overrides

`createReplayStore({ criticallyLowMs: 50_000 })` overrides buffer thresholds. This is how tests engineer specific buffer states without advancing hundreds of frames. For example:

- `criticallyLowMs: 50_000`: buffer is immediately "critically low" after a 15s fetch (forces stall behavior)
- `lowThresholdMs: 50_000`: buffer is always "low" (forces background prefetch)
- `criticallyLowMs: 0`: disable critically-low stalls entirely (isolate other behavior)

### Pure function tests

`format-time.test.ts` tests the `formatTime` utility directly, without Svelte runes or mocks. Use this as a template for adding tests for other pure functions.

---

## Browser tests

Browser tests use [`vitest-browser-svelte`](https://github.com/nicolo-ribaudo/vitest-browser-svelte) to render the full Svelte component in Chromium via Playwright. The Svelte `customElement` option is disabled in the browser test config so `render()` receives a standard component, not a custom element class.

### Setup helpers

| Helper | What it does | State after |
|--------|-------------|-------------|
| `mountPlayer()` | Renders the component, wires mocks, resolves `open()`, captures `PlayerApi` from the `ready` event | Fully initialized, `paused: true` |
| `mountPlayerPartial()` | Renders the component but does **not** resolve `open()` | Loading state. Use for testing loading UI or error injection. |

Both helpers return `{ screen, mockDataSource, mockWasm }`. `mountPlayer` also returns `{ api }`.

### Async fetch draining

Browser tests run in real time (no fake timers). After triggering a seek or play, pending data fetches must be drained manually:

```ts
// Drain all pending fetches in a polling loop.
await drainFetches(mockDataSource, makePdus(0, 30_000));

// Drain fetches until a specific promise settles (e.g., api.seek()).
await drainFetchesUntilSettled(mockDataSource, api.seek(15_000), makePdus(0, 30_000));

// Convenience: seek and drain in one call.
await seekAndDrain(api, mockDataSource, 15_000);

// Wait for a single pending fetch to appear, then resolve it.
await waitAndResolveFetch(mockDataSource, makePdus(0, 15_000));
```

Individual test files may compose higher-level helpers from these primitives. `seekAndDrain` and `waitAndResolveFetch` are defined locally in their respective test files.

### Pointer event dispatch

Pointer events are dispatched directly on the seekbar element (not `window`). The SeekBar component calls `setPointerCapture` on `pointerdown`, which routes all subsequent pointer events to the capturing element regardless of cursor position.

```ts
const seekbar = screen.container.querySelector('.__irp-seekbar')! as HTMLElement;

seekbar.dispatchEvent(new PointerEvent('pointerdown', { clientX, clientY, pointerId: 1, bubbles: true }));
seekbar.dispatchEvent(new PointerEvent('pointermove', { clientX: newX, clientY, pointerId: 1, bubbles: true }));
seekbar.dispatchEvent(new PointerEvent('pointerup',   { clientX: newX, clientY, pointerId: 1, bubbles: true }));
```

Note: programmatically dispatched events are untrusted, so `setPointerCapture` does not actually activate. The tests validate handler logic by dispatching directly on the element. Real pointer capture behavior is validated by the out-of-bounds drag test, which dispatches at coordinates beyond the element's bounding rect.

### Test workarounds

Some browser tests use synthetic DOM events instead of Playwright locator clicks due to test environment layout constraints (e.g., overlays intercepting clicks on the default 300x150 canvas). These are marked with brief comments in the test files.

---

## File structure

```
tests/
├── README.md                                  ← you are here
├── format-time.test.ts                        ← pure unit test for formatTime()
├── replay-store.svelte.test.ts                ← store state machine tests
├── helpers/
│   ├── mock-data-source.ts                    ← deferred-pattern ReplayDataSource mock
│   └── mock-wasm-replay.ts                    ← WasmReplayInstance + ReplayModule mock
└── browser/
    ├── setup.ts                               ← mountPlayer() / mountPlayerPartial() helpers
    ├── seek.browser.test.ts                   ← seek bar pointer interactions + keyboard seek
    ├── playback-controls.browser.test.ts      ← play/pause/reset buttons, canvas click, ended overlay
    ├── overlays.browser.test.ts               ← loading, buffering, action, ended overlay visibility
    └── speed-selector.browser.test.ts         ← speed popup open/close/select
```
