# IronRDP WTS Protocol Provider

Windows-specific Remote Desktop Services protocol provider implementation backed by IronRDP.

This crate is intended to host an `IWRdsProtocolManager`/`IWRdsProtocolListener`/`IWRdsProtocolConnection`
implementation for side-by-side protocol registration before any in-place replacement strategy.

## Status

Early implementation scaffold.

## Scope (current)

- Provider object model and session registry
- Listener lifecycle worker-thread dispatch
- Connection lifecycle state machine with transition validation
- CredSSP-first authentication bridge placeholders
- In-proc COM class factory and DLL exports (`DllGetClassObject`, `DllCanUnloadNow`)
- Side-by-side install and uninstall PowerShell scripts
- Side-by-side preflight and registration verification PowerShell scripts
- Side-by-side backup/restore and firewall helper scripts
- One-command first-run orchestrator script and mstsc `.rdp` file generator
- Read-only side-by-side smoke test script for go/no-go checks before mstsc
- Provider DLL build helper and orchestrator auto-detect/build options
- Listener port conflict detection and port-aware firewall rule naming
- Optional post-restart readiness wait helper (`TermService` + TCP listen state)

## Scope (next)

- Termsrv callback integration
- Input/video handle integration
- Licensing and virtual channel integration

## Protocol manager CLSID

`{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}`

The install scripts use this CLSID by default and wire it into both:

- `HKLM\SOFTWARE\Classes\CLSID\{...}\InprocServer32`
- `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\<Listener>\LoadableProtocol_Object`

## Side-by-side rollout

See [SIDEBYSIDE_SETUP.md](./SIDEBYSIDE_SETUP.md).
