# Windows TermService Protocol Provider

Experimental Windows Remote Desktop Services (`TermService`) protocol provider and companion server built on IronRDP.

It registers a side-by-side WTS protocol listener so IronRDP can drive authenticated Windows sessions alongside native `RDP-Tcp` for differential testing and protocol development.

## Layout

- [`crates/ironrdp-termsrv`](crates/ironrdp-termsrv/README.md): Companion service and session-local capture, cursor, input, clipboard, and virtual-channel helpers.
- [`crates/ironrdp-wtsprotocol-provider`](crates/ironrdp-wtsprotocol-provider/README.md): In-process COM protocol provider (`IWRdsProtocolManager`, `IWRdsProtocolListener`, and `IWRdsProtocolConnection`) loaded by `TermService`.
- [`crates/ironrdp-wtsprotocol-ipc`](crates/ironrdp-wtsprotocol-ipc): Shared control-pipe and display-configuration messages between the provider, companion, and display driver.
- [`crates/ironrdp-idd`](crates/ironrdp-idd/README.md): UMDF Indirect Display Driver (IddCx) used for remote session display attachment and dynamic resizing.

## Test VM

Use Windows with PowerShell 5.1 or 7, a Rust MSVC toolchain, and a Windows test VM with PSRemoting enabled.
Set `IRONRDP_TEST_VM_HOSTNAME`, `RDP_USERNAME`, and `RDP_PASSWORD` for the target before running deployment commands from the repository root.
The deployment account must be an administrator on the target.

```powershell
$env:IRONRDP_TEST_VM_HOSTNAME = 'rdp-test.example.com'
.\server\termsrv\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -WtsProvider -InstallIdd
```

The deployment stops TermService and may reboot the VM to enable test signing.
The target needs the RDS Session Host role.
Installing the optional IDD also requires the Windows Driver Kit and `psign-tool` on the build host; see the [driver setup guide][driver-setup].
Set `IRONRDP_DEPLOY_ROOT` or pass `-RemoteRoot` to change the remote deployment directory.
Set `IRONRDP_IDD_REMOTE_PATH` or pass `-IddRemotePath` to change the remote IDD package directory.
Cargo output paths honor the `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` environment variables.
The pinned public signing fixtures download on first use; pass `-TestCertificatesDirectory` to use a local directory containing `authenticode-test-ca.crt` and `authenticode-test-cert.pfx`.
Omit `-InstallIdd` when updating only the companion and provider.
The signing key stays local, and only the target trusts the public test certificates.
See [deployment options][deployment] for deferred startup and standalone mode.

Observe a connection through an isolated `ironrdp-agent` endpoint:

```powershell
.\server\termsrv\crates\ironrdp-termsrv\scripts\e2e-test-screenshot.ps1 -Mode Provider -SkipDeploy -StrictSessionProof
```

The E2E command uses the same hostname and credential environment variables.
It pins the target certificate only in a temporary local agent store and removes that store after the run.
The harness stops only its own client endpoint.
`-SkipDeploy` reuses installed binaries but still prepares IDD runtime state and a fresh graphics dump directory.
Session logoff/reset and Hyper-V power cycling require explicit opt-ins and exact targets.
Cleanup requires a desktop process with the requested account and domain; a short username from `quser` alone cannot authorize it.
Stale IDD devices are reported, not automatically removed.
Strict proof combines fresh client graphics with current-run logon, session, and IDD evidence; ambiguous or missing evidence fails.
The independent [session-local capture][capture] remains available to observe a connected desktop owned by the requested user.

Collect diagnostics without restarting services or changing registration:

```powershell
.\server\termsrv\scripts\collect-diagnostics.ps1
```

Its artifact directory contains provider data, an IDD archive, Type10 logon data, and `manifest.json`.
The manifest marks each section `ok`, `warning`, or `error`; partial collection preserves the successful sections.
Diagnostic files can contain usernames, addresses, and session data; review them before sharing.
The individual collectors and provider backup/restore commands remain independently callable.

## Helper tests

Run the local helper tests with Pester 5 or newer:

```powershell
Invoke-Pester -Path .\server\termsrv\tests
```

These tests do not deploy to or change the test VM.
They cover local orchestration and failure paths, not live driver loading, native service timing, or session capture.

[driver-setup]: crates/ironrdp-idd/README.md#set-up-a-test-vm
[deployment]: crates/ironrdp-termsrv/README.md#deploy-to-a-test-vm-psremoting
[capture]: crates/ironrdp-termsrv/scripts/capture-irdp-session-screenshot.ps1
