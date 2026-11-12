# CEF browser over IronRDP

A native CEF browser server with a Rust-drawn address bar and tabs.
CEF software view and popup paints go directly into Rust-owned BGRA surfaces, then through `ironrdp-server` to an RDP client.
No desktop capture, local browser window, CDP screencast, or HTML-based privileged toolbar is involved.

This is a local rendering and interaction prototype, **not a production RBI isolation boundary**.
It does not include containers, Devolutions Gateway, or RDM integration.

## Build

Use the repository's Rust toolchain, CMake, Ninja, and Python 3.9 or later.
Build on the intended operating system: `package-cef` is a native-host packager, not a cross-packager.
macOS needs the Xcode command-line tools; Windows needs the MSVC C++ build tools and Windows SDK; Linux needs a C++ toolchain and CEF's X11, GTK, NSS, GBM, and ALSA runtime libraries.
The package has its own Cargo workspace and lockfile, so root-workspace builds do not build or download CEF.
The pinned `tauri-apps/cef-rs` revision is `9aa78c450489eddb828eb982eaee6c104a89d348`, using CEF `156.4.0+156.0.26`.
Building the native feature downloads a large CEF distribution and compiles its wrapper.
The default `package-cef` command uses stock CEF.
The server requires patched CEF even when CLIPRDR redirection is disabled; it rejects stock CEF at browser startup.
The patches are in `server/cef/patches`:

| Patch | Tree | Purpose |
| --- | --- | --- |
| `isolated-clipboard.patch` | CEF | Process-private clipboard and its C API, including pasted files |
| `remote-devices-cef.patch` | CEF, after the clipboard patch | C API for remote camera, microphone, and WebAuthn |
| `remote-devices-chromium.patch` | Chromium | Embedder-fed capture devices and WebAuthn authenticator |
| `in-process-audio-shutdown.patch` | Chromium | Stops the in-process audio service before its `AudioManager` |
| `disable-chromium-shutdown-signals.patch` | Chromium, POSIX only | Lets Rust own shutdown cleanup |

Rebuild and distribute the patched CEF checkout, then set `CEF_PATH` to its distribution root before packaging.
The packager copies only regular files, so the distribution framework must not use symlinked `Versions` directories.
The custom build has been validated on macOS ARM64; Linux and Windows remain unverified.

Run commands from the repository root.
If CMake or Ninja is missing, install them locally on macOS or Linux:

```sh
python3 -m venv server/cef/dependencies/build-tools
server/cef/dependencies/build-tools/bin/pip install cmake ninja
export PATH="$PWD/server/cef/dependencies/build-tools/bin:$PATH"
```

On Windows, use the virtual environment's `Scripts` directory instead:

```powershell
py -m venv server/cef/dependencies/build-tools
.\server\cef\dependencies\build-tools\Scripts\python.exe -m pip install cmake ninja
$env:PATH = "$PWD\server\cef\dependencies\build-tools\Scripts;$env:PATH"
```

Build the application bundle and the test client:

```sh
cargo run --locked --manifest-path server/cef/Cargo.toml --features bundle --bin package-cef
cargo build --locked -p ironrdp-agent
```

The packager uses the matching upstream CEF packaging API.
Run the packaged executable so CEF can find its libraries, resources, locales, and subprocess entry point:

| Platform | Executable under `server/cef/artifacts/bundle/` | Native validation |
| --- | --- | --- |
| macOS | `ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server` | Apple Silicon build, real-agent acceptance, and private clipboard E2E |
| Linux | `ironrdp-cef-server` | Debian ARM64 VM browsing; private clipboard runtime unverified |
| Windows | `ironrdp_cef_server.exe` | ARM64 cross-build and browsing/download acceptance; private clipboard runtime unverified |

The macOS server is a background agent and does not appear in the Dock.
Its development bundle is not sealed: `codesign --verify` and Gatekeeper assessment fail, although the linker-signed executable runs locally.
Signing, notarization, and sandbox behavior under a properly signed bundle are not verified.

On Windows, the packager pairs CEF's console sandbox bootstrap with `ironrdp_cef_server.dll`, which exports `RunConsoleMain`.
The bootstrap handles both the browser and child processes and passes its sandbox context into CEF.
Do not substitute the bare Cargo binary or remove the sandbox to bypass a launch failure.
The Windows ARM64 test used a manually assembled bundle; native packaging and other Windows architectures remain unverified.
Windows binaries need the [Visual C++ runtime][vc-redist] for the target architecture.
The tested bundle supplied `VCRUNTIME140.dll` alongside the CEF executable.
The bootstrap changes its process's working directory; pass absolute artifact and font paths:

```powershell
$artifacts = Join-Path $PWD.Path "server/cef/artifacts/windows-run"
& .\server\cef\artifacts\bundle\ironrdp_cef_server.exe --bind-addr 127.0.0.1:3399 --url https://devolutions.net/ --artifacts $artifacts
```

On Linux, install a TrueType font such as DejaVu Sans and run as a non-root user with a working Chromium sandbox.
The server selects the headless Ozone backend and loads its sibling helper, resources, locales, and libraries; the executables use an `$ORIGIN` library search path.
With Xvfb and xauth installed, a private display can be started without a desktop session:

```sh
xvfb-run -a -s "-screen 0 1280x800x24 -nolisten tcp" \
  server/cef/artifacts/bundle/ironrdp-cef-server \
  --bind-addr 127.0.0.1:3399 --url https://devolutions.net \
  --artifacts server/cef/artifacts/linux-run
```

Do not add `--no-sandbox` when user namespaces or sandbox installation are unavailable; fix the native runtime configuration instead.
Running in Xvfb does not provide a container isolation boundary.
The Windows and Linux runtime and packager type-check with `cef/dox`, which skips native CEF linking.
Linux architectures other than ARM64 remain unverified.
The server chooses an installed platform font, or accepts an explicit TrueType file with `--font`.

## Run

```sh
server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --bind-addr 127.0.0.1:3399 \
  --url https://example.com \
  --artifacts server/cef/artifacts/manual
```

Each run needs a fresh artifact directory.
The server writes `server.addr`, `server.pem`, and `profile.path` there.
It creates a fresh browser profile shared by that run's tabs; disconnecting preserves the running browser, while restarting creates a different profile.
Use **Ctrl-C** for orderly shutdown and profile removal, or `--exit-after-seconds N` for a bounded run.
Unix also accepts SIGINT; the Windows console bootstrap also accepts Ctrl-Break.
Clients that support RDP error reporting receive a server-shutdown reason before disconnecting; others may report an unexpected disconnect.
A startup SIGINT is deferred until CEF initializes.
The startup mask assumes native library initializers do not leave a SIGINT-unblocked thread running before `main`.
On Windows, Ctrl-Break during CEF initialization can leave profile files locked until the browser process exits; the server reports a failure and leaves the remaining files in place.
The custom POSIX patch disables Chromium's own shutdown signal handler so Rust can close CEF and remove the profile on SIGINT.
An unpatched or stock CEF bundle may intercept POSIX shutdown signals before Rust cleanup.
macOS CEF teardown can also expire its Chromium watchdog before Rust removes the profile.
SIGTERM, crashes, stalled initialization or shutdown, and forced termination can leave a profile in the artifact directory; do not treat exit code zero alone as evidence of cleanup.
If CEF shutdown, a worker, or log preservation fails, the server logs and retains the profile.
A suspended CEF helper can outlive a successful server exit, so profile removal does not prove all helpers have stopped.

The listener only accepts loopback addresses and rejects a second simultaneous client.
It uses an ephemeral self-signed TLS identity and no credential authentication.
Keep it local; TLS here is not a substitute for a production authorization boundary.
The test runner computes the certificate's SHA-256 fingerprint independently and pins it in a dedicated agent certificate store.
It does not bypass certificate validation or modify the user's agent daemon.

The desktop starts at 1280x800.
Validated client desktop sizes and single-monitor resizes are supported from 320x240 through 3840x2160.
There are at most eight tabs.
Ctrl/Cmd+L, T, W, R, and Tab select the address, create/close/reload/cycle tabs; F5 and Alt+Left/Right navigate.
Mouse controls provide the same actions.
Unicode text input and a subset of US keyboard scancodes are supported, not full IME, international-layout, or keypad behavior.
The pinned macOS CEF build does not preserve the auto-repeat flag in synthetic key events.
On Linux ARM64, a test client that sends consecutive RDP key-downs produced DOM repeat events.
Windows repeat-flag delivery remains unverified with native CEF.

## Public websites

Use `--url https://devolutions.net/` to browse the public site through the RDP client.
Manual acceptance covered its homepage, scrolling, Products menu, Remote Desktop Manager page, back/forward navigation, and independent tabs with normal HTTPS verification.
The Linux ARM64 VM also displayed the homepage through the agent with normal HTTPS verification.
These checks are separate from the deterministic suite: public website availability and content must not become test dependencies.

## Private clipboard

The custom CEF build provides an in-memory clipboard for each browser process.
The server installs it before browser startup, so page reads and writes never use the host clipboard.
With `--clipboard`, the CLIPRDR bridge redirects UTF-8 text, HTML, and PNG in both directions between the browser and RDP client.
Without RDP redirection, the private clipboard remains local to the browser process.
Stock CEF lacks the private API and is rejected at startup rather than falling back to the platform clipboard.
The macOS ARM64 end-to-end test covers all three formats and confirms that the host pasteboard change count stays unchanged.
The private clipboard itself does not use X11 or a desktop clipboard service.
The Linux runtime selects headless Ozone but has only been tested under Xvfb; private clipboard operation and rendering without Xvfb remain unverified.

## Remote devices

The patched runtime never enumerates host cameras, microphones, or authenticators.
The server always starts CEF with `remote-media-capture`, which replaces capture devices with one camera and one microphone fed through the CEF C API.
It also passes `remote-webauthn`, which sends every Web Authentication request to one embedder authenticator as raw CTAP2 and suppresses Chromium's authenticator UI.
The audio and video capture services run in the browser process so the embedder can feed them.
Echo cancellation stays in the renderer, and macOS camera frames use shared memory instead of IOSurfaces.

No RDP channel feeds these devices yet.
Without a feeder, the media permission handler denies camera and microphone requests, and WebAuthn requests fail as if no authenticator answered.
Desktop capture is always denied, so pages cannot capture the host display.
Pasted files require absolute paths readable by the browser; CLIPRDR file transfer does not stage them yet.

For tests, build with `--features device-simulator` and pass `--simulate-devices`.
The simulator streams a green camera image and a 440 Hz tone, answers WebAuthn with a test CTAP2 authenticator, and replaces each client clipboard copy with one staged file.
Its credentials are not secure; never enable it in a deployment.

## Redirected downloads

Enable the server with `--downloads` and start the agent daemon with `--rdpdr-download-dir /absolute/download/folder`.
For a visible client, `ironrdp-viewer --rpc --rdpdr-download-dir /absolute/download/folder` uses the same restricted device.
On Windows, use an absolute drive-letter path without reparse points, not a UNC or verbatim path.
The folder must already exist, and untrusted local processes must not be able to modify the folder or hard-link its files.
It is announced as the download-only RDPDR filesystem device `DL`; the browser server never receives its physical path.
Existing files cannot be read, listed, changed, or overwritten through this device.
Native file dialogs and browser uploads remain disabled.
On Windows, connections exposing this device reject other device redirection; ordinary drive redirection is not an equivalent restricted sink.

Downloads are saved automatically, with collision-safe filenames.
CEF first writes into the private browser profile; a background thread then streams bounded RDPDR writes into a new temporary client file.
Flush, atomic non-replacing rename, and successful close are required before the Rust status bar reports completion.
If the rename acknowledgment is lost, or the following close fails or is unacknowledged, the client file may already exist even though delivery reports failure.
The status bar separates browser and client byte counts and provides **Cancel all** without covering browser content.
Closing a tab navigates its page to `about:blank` while its active downloads continue; **Cancel all** still cancels them.
The browser accepts a before-unload prompt for the closed tab, but a hung renderer can delay page termination.
Tab closure is not a security boundary and does not guarantee immediate network shutdown or stop service workers.
A concurrent download failure remains visible while other transfers finish.
Completed client files survive disconnect and browser shutdown; normal cancellation and teardown attempt to remove unpublished client and staging files.
Cleanup failures are logged and can leave incomplete files; cleanup is not crash recovery.

Defaults are 1 GiB per file, 4 GiB per browser run, and two concurrent downloads.
Configure them with `--download-max-file-bytes`, `--download-max-session-bytes`, and `--download-max-concurrent`.
File limits must be 1 byte through 1 GiB, concurrency must be one or two, and the session limit must be at least the file limit.
Failed and cancelled downloads consume the bytes already observed; reconnecting does not reset that browser run's budget.
CEF callbacks can overshoot a limit before cancellation, so these checks prevent publication of oversized files but are not a hard staging-disk quota.

Missing `DL` devices and rejected transfers produce explicit errors rather than falling back to the host Downloads folder.
Delivery failures fence the RDP connection.
Disconnecting with an unacknowledged filesystem request stops the browser server, preventing a queued request from reaching another connection; restart it afterward.
Removing the download device disconnects the session.
RDPDR reinitialization cancels downloads from the previous device epoch; an unacknowledged request also stops the server.
Idle reconnects, completed downloads, and disconnects during browser-side downloading do not require that fallback.
The browser server rejects inbound RDPDR messages larger than 1 MiB before reassembly, including when downloads are disabled.
Use the [native download policy][download-policy] for client-side limits and filesystem assumptions.

## End-to-end test

```sh
python3 server/cef/testing/e2e.py \
  --server-bin server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --agent target/debug/ironrdp-agent
```

The runner starts a private loopback HTTP fixture, server, and agent daemon.
It drives the browser exclusively through RDP mouse, scancode, and Unicode input.
Decoded agent screenshots check page colors, clicks, physical key identity and case, exact Unicode text and Backspace, and browser focus after leaving address editing.
They also check cookies, local storage, IndexedDB write/reopen/read, select-popup composition and dismissal, scrolling, address navigation, history, reload, keyboard/mouse tabs, resizing, and reconnect.
The runner checks repeated text entry, but the agent inserts a release between repeated presses and cannot verify DOM key-repeat flags.
A second server run checks that prior cookies and local storage are absent.
The runner also checks server exit status and profile removal.
On Linux or Windows, substitute the packaged executable and native agent paths.
Windows uses a unique named pipe and sends Ctrl-Break to a new console process group; Unix uses a private socket and SIGINT.
Screenshot color comparisons allow four levels per RGB channel because the negotiated RemoteFX codec is lossy; geometry and distinct fixture states are still checked.

Run the private clipboard test against the custom macOS bundle and verify that stock CEF fails closed:

```sh
python3 server/cef/testing/clipboard.py \
  --server-bin server/cef/artifacts/clipboard-bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --stock-server-bin server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --agent target/debug/ironrdp-agent
```

The runner checks text, HTML, and PNG in both directions, unchanged host pasteboard state, and profile cleanup.

Check remote devices with a simulator bundle built against the patched CEF:

```sh
cargo run --locked --manifest-path server/cef/Cargo.toml \
  --features bundle,device-simulator --bin package-cef -- artifacts/devices-bundle
python3 server/cef/testing/devices.py \
  --server-bin server/cef/artifacts/devices-bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --agent target/debug/ironrdp-agent
```

The runner checks WebAuthn registration, camera pixels, microphone level, desktop-capture denial, a pasted file, and clean shutdown.
It serves the fixture from `localhost` because WebAuthn rejects IP-address origins.
Reports and logs are under `server/cef/artifacts/devices-e2e/run-*/`.

Evidence is written under `server/cef/artifacts/e2e/run-*/`: `report.json`, screenshots, agent command/output logs, and per-server logs and certificates.
On Unix, CEF logs are copied out of the profile on orderly shutdown; on Windows, CEF writes directly to `cef.log` in the artifact directory.
Logs and profiles may contain visited URLs or site data; treat artifacts accordingly.
The runner uses a short, private `.cef-*` IPC directory to stay below macOS Unix-socket path limits and removes it afterward.

Run rendering and redirected-download acceptance together:

```sh
python3 server/cef/testing/downloads.py \
  --server-bin server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --agent target/debug/ironrdp-agent
```

This runner selects its own private client directory and enables downloads with small test limits.
It retains the rendering checks and compares client bytes and SHA-256 hashes against independent loopback HTTP payloads.
Coverage includes binary, empty, Unicode, duplicate, redirected, unknown-length, and multi-chunk downloads; concurrent transfers; exact file/session limits; interrupted responses; upload-dialog refusal; cancellation; missing or rejected devices; refused client writes; closing a download's tab and unloading its page; disconnects during browser downloads and RDPDR writes; and SIGINT during a browser download.
It checks terminal RDPDR acknowledgments, temporary-file cleanup, profile removal, persistence of completed files, and a fresh connection after interruption.
The Windows write-denial case temporarily changes the test directory's ACL and restores it afterward.
Status-bar-visible checks cover new tabs, desktop resizing, popup composition, and the browser's bottom-most controls.
The browser server's handling of refused rename or close responses and acknowledgement timeouts is checked with a controlled RDPDR adapter, not a real agent.
Reports, screenshots, command logs, and per-server logs are under `server/cef/artifacts/download-e2e/run-*/`.
Use `--skip-baseline` only when rerunning the download cases independently.

To verify a download through the visible RPC viewer, build `ironrdp-viewer` and run:

```sh
python3 server/cef/testing/viewer_download.py \
  --server-bin server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --agent target/debug/ironrdp-agent \
  --viewer target/debug/ironrdp-viewer
```

The viewer test also checks a pre-connection resize, resize after reconnecting, and a client-initiated disconnect.
It has passed on macOS and Windows ARM64.

Check the client download boundary through real RDPDR requests from a separate test server:

```sh
cargo build --locked --manifest-path server/cef/Cargo.toml --features runtime --bin rdpdr-policy-fixture
python3 server/cef/testing/rdpdr_policy.py \
  --server-bin server/cef/target/debug/rdpdr-policy-fixture \
  --agent target/debug/ironrdp-agent
```

The fixture attempts reads, listing, device controls, mutation of existing files, traversal, links, replacement, unowned handles, and post-publication writes and deletion.
It verifies the protocol replies, including the exclusive-create information field, and the client files left behind.
It also replaces the selected directory path after daemon startup and verifies that writes remain in the pinned directory.
Reports and logs are under `server/cef/artifacts/rdpdr-policy-e2e/run-*/`.

Run repeated single-interrupt shutdowns with `--phase startup`, `immediate`, or `ready`:

```sh
python3 server/cef/testing/shutdown.py \
  --server-bin server/cef/artifacts/bundle/ironrdp-cef-server.app/Contents/MacOS/ironrdp-cef-server \
  --phase ready --iterations 100
```

Each iteration checks the exit status, profile removal, and shutdown of CEF helpers using that profile.
The `ready` phase waits for a helper before signaling the server.
Reports and logs are under `server/cef/artifacts/shutdown-stress/run-*/`.
On macOS or Linux, replace `--iterations 100` with `--iterations 1 --fail-log-copy` to check profile retention when the CEF log cannot be copied.
The probe saves the retained log and removes its test profile after the helpers stop.
On Windows, use the packaged executable and add `--hold-profile-lock` in the `ready` phase to check retrying a temporary sharing violation.

The pure shell/framebuffer tests do not require a native CEF build:

```sh
cargo test --locked --manifest-path server/cef/Cargo.toml
```

Check native targets with Clippy after building the bundle:

```sh
cargo clippy --locked --manifest-path server/cef/Cargo.toml \
  --features bundle --all-targets -- -D warnings
```

## Boundaries and limitations

CEF initialization, message pumping, callbacks, and browser commands stay on the main/UI thread.
Borrowed paint buffers are copied before callbacks return.
Complete per-tab surfaces are composed with clipped popup surfaces and Rust chrome; the RDP thread receives only the latest complete desktop frame.
Frames are rate-limited to 30 Hz, dimensions and callback/input queues are bounded, and queue exhaustion during normal operation is fatal rather than silently dropping key or button transitions.
The implementation sends full frames rather than tracking CEF damage rectangles.

The helper initializes CEF's macOS sandbox, and the server does not disable browser web security, website certificate validation, or renderer sandboxing.
In a Linux ARM64 VM, compositor-bearing CEF children ran unprivileged with seccomp and separate user and PID namespaces.
In a Windows 11 ARM64 VM, observed renderers had untrusted-integrity tokens and belonged to a job, but were not AppContainers.
These observations do not enumerate every child or prove container isolation.
GPU rendering, remote debugging, native context menus, and external-protocol launches are disabled.
Downloads are disabled unless explicitly enabled and require the dedicated client device.
CEF's process-private clipboard is always active; CLIPRDR redirection is opt-in.
Audio output is muted, and no audio RDP backend is installed.
This isolates page clipboard operations from the host pasteboard but does not virtualize the host filesystem, network, keychain, permission prompts, or file pickers.
On macOS, Chromium can read a [host Chrome brand][chromium-brand] and send an [RLZ request][chromium-rlz] that may include a machine identifier.
Only HTTP, HTTPS, and `about:blank` navigations are supported; allowed new-window requests become tabs.
Media and WebAuthn permissions are decided without prompts; other permission prompts and arbitrary websites are not covered by the fixture.

The `UNTESTED:` property at the resize bridge is CEF's UI-thread paint ordering across rapid size changes.
Queued paints carry Rust generations and wrong-size frames are rejected, but CEF does not provide a native resize token for a late paint after dimensions cycle back.
Recheck that ordering when changing the CEF version or message pump.

A fresh profile and Chromium's renderer sandbox do not replace container hardening, network policy, tenant isolation, resource enforcement, authorization, or secure teardown for a deployed RBI service.

[download-policy]: ../../crates/ironrdp-rdpdr-native/README.md
[chromium-brand]: https://github.com/chromium/chromium/blob/8954d18543712dc11b97c3aca0a6e714aa2d7dcd/chrome/browser/google/google_brand_mac.mm
[chromium-rlz]: https://github.com/chromium/chromium/blob/8954d18543712dc11b97c3aca0a6e714aa2d7dcd/rlz/lib/financial_ping.cc
[vc-redist]: https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist
