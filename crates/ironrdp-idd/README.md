# ironrdp-idd

`ironrdp-idd` is the IronRDP Indirect Display Driver (IDD) scaffold.
Windows Remote Desktop Services loads it through the WTS protocol provider in WDDM IDD mode.

This crate is intentionally *safe-by-default* for normal workspace builds:

- It does **not** require the Windows Driver Kit (WDK) to be installed to run `cargo check` / clippy.
- IddCx linking and callback argument layouts that depend on WDK headers are **opt-in**.

## Build knobs

- `IRONRDP_IDD_LINK=1`
  - Enables IddCx stub linking (`iddcxstub` + `d3d11`/`dxgi`) from [build.rs](build.rs).
  - Also enables a compile-time cfg (`ironrdp_idd_link`) for code that calls into the IddCx function table.
  - This typically requires the WDK import libraries to be available to the MSVC toolchain.

- `IRONRDP_IDDCX_LIB_DIR=C:\path\to\folder\containing\iddcxstub.lib`
  - Optional override used by [build.rs](build.rs) when `IRONRDP_IDD_LINK=1` is set.

- `IRONRDP_WINDOWS_KITS_ROOT=C:\Program Files (x86)\Windows Kits\10`
  - Optional override used by [build.rs](build.rs) to locate `iddcxstub.lib` under `<root>\Lib\*\{um,km}\<arch>\iddcx\*`.

- `IRONRDP_WDF_UMDF_STUB_VERSION=2.33`
  - Optional override for selecting a specific `WdfDriverStubUm.lib` version under `<root>\Lib\wdf\umdf\<arch>\<version>`.
  - Defaults to `2.33` to match the WDF dispatch symbol used by the crate.

- `IRONRDP_WDF_USE_LOCAL_STUB=1`
  - Optional switch to force use of bundled [WdfDriverStubUm.lib](WdfDriverStubUm.lib) instead of WDK-discovered stubs.
  - By default (`0`/unset), [build.rs](build.rs) prefers WDK stubs and falls back to the bundled copy only when needed.

- `IRONRDP_WDF_STUB_BUILD_NUMBER=26100`
  - Optional override for the bind-info build number patch applied to the selected WDF stub.
  - The selected WDF stub is linked through a patched copy in `OUT_DIR` to avoid pre-release WDK bind-info rejection.

- Feature `iddcx-experimental-layout`
  - Enables an **experimental** `repr(C)` layout for `IDARG_IN_SETSWAPCHAIN` used by the monitor swapchain callbacks.
  - Off by default because the public documentation does not guarantee these layouts.

- Machine runtime state file: `C:\ProgramData\IronRDP\Idd\runtime-config.txt`
  - The driver reads diagnostic settings such as `dump_dir`, `hardware_id`, and `active_video_source` from this file instead of inheriting process environment variables.
  - Session ownership comes from the device's OS-owned `DEVPKEY_Device_SessionId`, never the shared file's `session_id` or readiness flags.
  - The swapchain thread performs best-effort CPU readback only when the device is bound to a remote session and a dump directory is configured.
  - Dump files are written as `idd-swapchain-s####-f##########-######.bmp`, which lets the e2e harness correlate frame artifacts with the target session.
- The INF declares `UmdfExtensions=IddCx0102`, the only service name IddCx registers, even though the driver binds IddCx 1.4.
  - Declaring `IddCx0104` makes IddCx's `WdfDriverCreate` fail with `STATUS_INVALID_PARAMETER` and the session drop with `ERRINFO_CLOSE_STACK_ON_DRIVER_FAILURE`.
  - Remote-session strict e2e assumes the package advertises the IddCx 1.4 remote-IDD contract and `UmdfHostProcessSharing=ProcessSharingDisabled`.

The diagnostic DOS aliases are shared across device instances.
An alias already owned by another instance must not abort device startup; UMDF can report this collision as `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)` rather than `STATUS_OBJECT_NAME_COLLISION`.
With `IRONRDP_IDD_LINK=1` and the WDK configured, run `cargo test --locked -p ironrdp-idd --lib` to exercise the device-add callback against simulated framework responses.

The companion publishes session-specific display requests in `C:\ProgramData\IronRDPDisplay`, separately from the diagnostic runtime files.
Each driver host reads only its device session's requests, so another user's resize or disconnect cannot redirect its display updates.
`install-idd-remote.ps1` provisions this directory with write access restricted to SYSTEM and administrators and read access for LocalService and NetworkService.
The driver updates target modes before applying a new display configuration and does not resubmit an unchanged request on every frame or inactive-path notification.
Adapter initialization completes only in `EvtIddCxAdapterInitFinished`; power entry does not poll for it.
Display updates run in a WDF timer parented to the monitor, so framework cleanup drains them before destroying the monitor and adapter.
Power exit closes admission and stops the timer without waiting for a native display call.
IddCx 1.10 or newer enables arbitrary monitor-compatible target modes; older runtimes retain the legacy adapter flags.
See [desktop resizing][termsrv-resize] for supported dimensions and capture behavior.

## Scripts

The UMDF binary [runs from the Driver Store][driver-store] using DIRID 13, so package updates do not overwrite a shared system DLL.
Check the loaded `WUDFHost.exe` module path when verifying deployment; a legacy copy under `System32\drivers\UMDF` can remain after an upgrade.
Driver Store packages cannot be force-deleted while installed on a device; the installer uses `pnputil /delete-driver /uninstall`.

All scripts live in [crates/ironrdp-idd/scripts](scripts).

- [deploy-idd.ps1](scripts/deploy-idd.ps1): enable test signing + install Devolutions test certificates on the target VM.
- [verify-test-signing.ps1](scripts/verify-test-signing.ps1): verify `bcdedit` test signing is enabled on the VM.
- [sign-driver.ps1](scripts/sign-driver.ps1): stage the built Rust DLL as `IronRdpIdd.dll`, sign it, and generate/sign the catalog file (requires Windows SDK/WDK tools).
- [install-idd-remote.ps1](scripts/install-idd-remote.ps1): copy `IronRdpIdd.dll` + INF/CAT to the VM and install via `pnputil`.
- [restart-rds-services.ps1](scripts/restart-rds-services.ps1): restart `TermService` and the IronRDP companion service.
- [restart-vm.ps1](scripts/restart-vm.ps1): reboot the VM and wait for WinRM to come back (use after enabling test signing).
- [collect-idd-diagnostics.ps1](scripts/collect-idd-diagnostics.ps1): collect driver/device + event log diagnostics into a zip.
- [ensure-rdsh-role.ps1](scripts/ensure-rdsh-role.ps1): check (and optionally install) the `RDS-RD-Server` role on Windows Server.
- [check-windows-edition.ps1](scripts/check-windows-edition.ps1): check Windows edition suitability for 3rd party protocol providers.
- [find-wdk-tools.ps1](scripts/find-wdk-tools.ps1): locate `iddcx.h`, `iddcxstub.lib`, and WDK/SDK signing tools on the machine.

## Set up IT-HELP-TERMSRV

Run from the repository root with `RDP_USERNAME` and `RDP_PASSWORD` supplied through the environment.
The scripts default to `IT-HELP-TERMSRV` and use HTTPS remoting on port 5986 when HTTP authentication is unavailable.
The lab HTTPS fallback skips certificate validation without changing local TrustedHosts or certificate stores.

```powershell
.\crates\ironrdp-idd\scripts\ensure-rdsh-role.ps1 -AttemptInstall
.\crates\ironrdp-idd\scripts\deploy-idd.ps1
.\crates\ironrdp-idd\scripts\restart-vm.ps1
$env:IRONRDP_IDD_LINK = '1'
cargo build --release --locked -p ironrdp-idd
.\crates\ironrdp-idd\scripts\sign-driver.ps1
.\crates\ironrdp-idd\scripts\install-idd-remote.ps1
```

Install the WDK and `psign-tool` on the build machine first.
For an extracted WDK, set `IRONRDP_WINDOWS_KITS_ROOT` to its `c` directory before building and signing.
Test signing requires a compatible boot policy; a Secure Boot policy that blocks it must be resolved before installation.

Signing uses the public test-only fixtures pinned to commit `2a7a928a7db8479ca4c96273579aac8c85f60a09` in [Devolutions/psign][psign-fixtures].
The helper caches them under `%LOCALAPPDATA%\IronRDP\TestSigning` and exports a public signer certificate.
`psign-tool` 0.8.0 needs a persisted key provider, so the signing script temporarily imports the PFX into `CurrentUser\My` and removes the imported certificate and key afterward.
It preserves a pre-existing personal-store certificate and never imports local trust anchors.
WDK Inf2Cat generates the driver catalog; `psign-tool` signs the DLL and catalog, then verifies both signatures with an explicit CA anchor and checks DLL catalog membership.
Offline signature verification does not establish Windows driver-policy acceptance.

Only the VM receives the public CA and signer certificates in `LocalMachine\Root` and `LocalMachine\TrustedPublisher`.
The PFX and private key stay on the build machine; these public fixtures must never be used for production signing.
Use `ironrdp-agent`, not `mstsc` or MsRdpEx, for VM RDP connections; see the [TermSrv connection instructions][termsrv-rdp].

## Current status

- Swapchain processing is a thread lifecycle scaffold (waits on the “new frame” event, uses a terminate event for shutdown, and enables MMCSS best-effort).
- Optional diagnostics now flow through `C:\ProgramData\IronRDP\Idd\runtime-config.txt` and `C:\ProgramData\IronRDP\Idd\ironrdp-idd-debug.log`, which are also collected by the strict e2e harness.
- Real IddCx swapchain acquire/release calls and accurate callback arg layouts require WDK headers/import libs.
  - Use [find-wdk-tools.ps1](scripts/find-wdk-tools.ps1) to confirm `iddcx.h`, `iddcxstub.lib`, and `Inf2Cat.exe` are present.

[psign-fixtures]: https://github.com/Devolutions/psign/tree/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode
[driver-store]: https://learn.microsoft.com/en-us/windows-hardware/drivers/develop/run-from-driver-store
[termsrv-rdp]: ../ironrdp-termsrv/README.md#connect-with-ironrdp-agent
[termsrv-resize]: ../ironrdp-termsrv/README.md#desktop-resizing
