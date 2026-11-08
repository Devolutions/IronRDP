# ironrdp-idd

`ironrdp-idd` is the experimental IronRDP Indirect Display Driver (IDD).
Windows Remote Desktop Services loads it through the WTS protocol provider in WDDM IDD mode.

This crate is intentionally *safe-by-default* for normal workspace builds:

- It does **not** require the Windows Driver Kit (WDK) to be installed to run `cargo check` / clippy.
- IddCx linking and callback argument layouts that depend on WDK headers are **opt-in**.

## Build knobs

- `IRONRDP_IDD_LINK=1`
  - Enables IddCx stub linking (`iddcxstub` + `d3d11`/`dxgi`) from [build.rs](build.rs).
  - Also enables a compile-time cfg (`ironrdp_idd_link`) for code that calls into the IddCx function table.
  - This requires the WDK import libraries to be available to the MSVC toolchain.

- `IRONRDP_IDDCX_LIB_DIR=C:\path\to\folder\containing\iddcxstub.lib`
  - Optional override used by [build.rs](build.rs) when `IRONRDP_IDD_LINK=1` is set.

- `IRONRDP_WINDOWS_KITS_ROOT=C:\Program Files (x86)\Windows Kits\10`
  - Optional override used by [build.rs](build.rs) to locate `iddcxstub.lib` under `<root>\Lib\*\{um,km}\<arch>\iddcx\*`.

- `IRONRDP_WDF_UMDF_STUB_VERSION=2.33`
  - Optional override for selecting a specific `WdfDriverStubUm.lib` version under `<root>\Lib\wdf\umdf\<arch>\<version>`.
  - Defaults to `2.33` to match the WDF dispatch symbol used by the crate.

- `IRONRDP_WDF_UMDF_LIB_DIR=C:\path\to\folder\containing\WdfDriverStubUm.lib`
  - Optional override for the WDK UMDF stub library directory.

- `IRONRDP_WDF_STUB_BUILD_NUMBER=26100`
  - Optional override for the bind-info build number patch applied to the selected WDF stub.
  - The selected WDF stub is linked through a patched copy in `OUT_DIR` to avoid pre-release WDK bind-info rejection.
  - `IRONRDP_IDD_LINK=1` requires this WDK library; normal `cargo check` and Clippy builds do not.

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

All scripts live in [server/termsrv/crates/ironrdp-idd/scripts](scripts).

- [deploy-idd.ps1](scripts/deploy-idd.ps1): enable test signing + install Devolutions test certificates on the target VM.
- [sign-driver.ps1](scripts/sign-driver.ps1): stage the DLL, INF, and signed catalog under Cargo's target directory in `termsrv\idd` (requires Windows SDK/WDK tools).
- [install-idd-remote.ps1](scripts/install-idd-remote.ps1): copy `IronRdpIdd.dll` + INF/CAT to the VM and install via `pnputil`.
- [restart-rds-services.ps1](scripts/restart-rds-services.ps1): restart the deployment-owned companion before `TermService`.
  Pass the deployment's `-RemoteRoot` and `-TaskName` when they differ from the configured defaults.
- [restart-vm.ps1](scripts/restart-vm.ps1): reboot the VM and wait for WinRM to come back (use after enabling test signing).
- [collect-idd-diagnostics.ps1](scripts/collect-idd-diagnostics.ps1): collect driver/device + event log diagnostics into a zip.
- [ensure-rdsh-role.ps1](scripts/ensure-rdsh-role.ps1): check (and optionally install) the `RDS-RD-Server` role on Windows Server.
- [find-wdk-tools.ps1](scripts/find-wdk-tools.ps1): locate `iddcx.h`, `iddcxstub.lib`, and WDK/SDK signing tools on the machine.

Set `IRONRDP_WINDOWS_KITS_ROOT` when the Windows Kits directory is outside its default location.

## Set up a test VM

Run from the repository root with `IRONRDP_TEST_VM_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD` supplied through the environment.
The remoting helper tries HTTP first, then HTTPS on port 5986 when HTTP authentication fails.
HTTPS remoting validates the server certificate.
Configure a trusted certificate on the VM before using this fallback.

```powershell
$env:IRONRDP_TEST_VM_HOSTNAME = 'rdp-test.example.com'
.\server\termsrv\crates\ironrdp-idd\scripts\ensure-rdsh-role.ps1 -AttemptInstall
.\server\termsrv\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -WtsProvider -InstallIdd
```

The deployment command builds and signs locally, provisions target trust, reboots when test-signing configuration changes, and installs the driver before starting the companion and TermService.
Role installation may require a separate reboot.
The individual signing, installation, diagnostics, and recovery scripts remain available.

Install the WDK and `psign-tool` on the build machine first.
For an extracted WDK, set `IRONRDP_WINDOWS_KITS_ROOT` to its `c` directory before building and signing.
Test signing requires a compatible boot policy; a Secure Boot policy that blocks it must be resolved before installation.

Signing uses the public test-only fixtures pinned to commit `2a7a928a7db8479ca4c96273579aac8c85f60a09` in [Devolutions/psign][psign-fixtures].
The helper caches them under `%LOCALAPPDATA%\IronRDP\TestSigning` and exports a public signer certificate.
For an offline setup, pass `-TestCertificatesDirectory` to the deployment command with a writable directory containing `authenticode-test-ca.crt` and `authenticode-test-cert.pfx`.
`psign-tool` 0.8.0 needs a persisted key provider, so the signing script temporarily imports the PFX into `CurrentUser\My` and removes the imported certificate and key afterward.
It preserves a pre-existing personal-store certificate and never imports local trust anchors.
WDK Inf2Cat generates the driver catalog; `psign-tool` signs the DLL and catalog, then verifies both signatures with an explicit CA anchor and checks DLL catalog membership.
Offline signature verification does not establish Windows driver-policy acceptance.

Only the VM receives the public CA and signer certificates in `LocalMachine\Root` and `LocalMachine\TrustedPublisher`.
The PFX and private key stay on the build machine; these public fixtures must never be used for production signing.
Use `ironrdp-agent`, not `mstsc` or MsRdpEx, for VM RDP connections; see the [TermSrv connection instructions][termsrv-rdp].

Swapchain processing acquires and releases IddCx buffers, waits for new frames, and optionally reads frames back for diagnostics.
Runtime traces are written to `C:\ProgramData\IronRDP\Idd\ironrdp-idd-debug.log`.
Use [find-wdk-tools.ps1](scripts/find-wdk-tools.ps1) to confirm that the required WDK headers, import libraries, and `Inf2Cat.exe` are present.

[psign-fixtures]: https://github.com/Devolutions/psign/tree/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode
[driver-store]: https://learn.microsoft.com/en-us/windows-hardware/drivers/develop/run-from-driver-store
[termsrv-rdp]: ../ironrdp-termsrv/README.md#connect-with-ironrdp-agent
[termsrv-resize]: ../ironrdp-termsrv/README.md#desktop-resizing
