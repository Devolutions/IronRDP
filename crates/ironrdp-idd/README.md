# ironrdp-idd

`ironrdp-idd` is the IronRDP Indirect Display Driver (IDD) scaffold. It is intended to be loaded by Windows Remote Desktop Services (via the WTS protocol provider) in WDDM IDD mode.

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

## Scripts

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

## Current status

- Swapchain processing is a thread lifecycle scaffold (waits on the “new frame” event, uses a terminate event for shutdown, and enables MMCSS best-effort).
- Real IddCx swapchain acquire/release calls and accurate callback arg layouts require WDK headers/import libs.
  - Use [find-wdk-tools.ps1](scripts/find-wdk-tools.ps1) to confirm `iddcx.h`, `iddcxstub.lib`, and `Inf2Cat.exe` are present.