# Windows Sandbox reversing notes

This directory records the Windows Sandbox investigation that informed the experimental
[`ironrdp-wsb`](../../crates/ironrdp-wsb) crate and the Windows Sandbox support in
[`ironrdp-agent`](../../crates/ironrdp-agent).

These are engineering notes, not a Windows API contract. The relevant Microsoft components are
private and version-sensitive. Findings are limited to the tested Windows 11 `10.0.26100` component
family and distinguish observed behavior from static-analysis inference.

## Scope and safety boundary

The investigation establishes how IronRDP can use supported product paths and the private UDK
lifecycle ABI already exposed by Windows Sandbox. It does **not** modify or document ways to evade
Windows licensing, session limits, package identity, access-control checks, or other product
enforcement.

The notes distinguish the host LSM admission result from its wire manifestations:
`ERROR_REMOTE_SESSION_LIMIT_EXCEEDED`, `ERRINFO_LOGOFF_BY_USER`, and an occasional
`CloseStackOnDriverFailure` teardown race. None is an IronRDP setting or supported tuning surface.

## Reading order

| Note | Purpose |
| --- | --- |
| [Windows Sandbox architecture](windows-sandbox-architecture.md) | Component map and the relationship between product orchestration, VM hosting, RDP, and licensing |
| [Direct VM lifecycle](windows-sandbox-direct-lifecycle.md) | Official server path versus the experimental UDK path used by `ironrdp-wsb` |
| [RDP and RDV transport](windows-sandbox-rdp-transport.md) | Named-pipe, VMBus, RDV, and host/guest transport relationships |
| [Guest RDP server](windows-sandbox-guest-rdp-server.md) | The hard-coded type-2 listener, `rdpcorets.dll`, `vmbuspipe.dll`, and the shared RDP connection path |
| [SynthRDP control handoff](windows-sandbox-synthrdp-control.md) | Private VMBus listener state machine, fixed control endpoint, session data-channel handoff, and generic pipe boundary |
| [RDP protocol boundary](windows-sandbox-rdp-protocol-boundary.md) | The documented RDP PDU layer versus the private named-pipe, RDV, and VMBus layers |
| [Enhanced Mode virtual devices](windows-sandbox-enhanced-mode.md) | `vmuidevices.dll` and `rdp4vs.dll`, a separate Hyper-V RDP stack |
| [Licensing and desktop admission](windows-sandbox-licensing.md) | Verified guest-side licensing route, observed decisions, and unresolved host-side policy receiver |
| [Component and registration catalog](windows-sandbox-component-catalog.md) | Attributable processes, binaries, VDEVs, registrations, interfaces, and their non-overlapping roles |
| [Evidence and version matrix](windows-sandbox-evidence.md) | Evidence classes, examined binaries, version scope, and remaining attribution work |
| [Open questions and evidence plan](windows-sandbox-open-questions.md) | Unresolved boundaries, the narrow evidence needed to answer them, and investigation safety constraints |

## High-level result

```mermaid
flowchart LR
    WSS[WindowsSandboxServer.exe<br/>product orchestration] --> PRODUCTVM[Product-private VM provisioning]
    UDK --> MVM[ManagedWindowsVM.exe<br/>out-of-process WinRT server]
    MVM --> CMS[Private Container Manager backend]
    PRODUCTVM --> VM[Sandbox VM / vmwp.exe<br/>per-VM worker]
    CMS --> VM

    VM --> PIPE[\\.\pipe\{VM-ID}<br/>owned by vmwp.exe]
    VM --> SYNTH[vmuidevices.dll + rdp4vs.dll<br/>worker RDP/display bridge]
    SYNTH --> PIPE
    GUEST[Sandbox guest] --> TERMSRV[termsrv.dll]
    TERMSRV --> LIC[tssrvlic.dll + LSCSHostPolicy.dll]
    LIC <--> RDV[Generic RDV/RIM transport]
```

There are two separately evidenced RDP-related paths:

- The **guest VM-listener path** is statically established: `termsrv.dll` configures a type-2
  VMBus listener that reaches guest built-in licensing and the LSCS/RDV bridge.
- The **worker VM-ID pipe path** is dynamically established: the Sandbox `vmwp.exe` worker owns
  `\\.\pipe\{VM-ID}` and loads `vmuidevices.dll`, `rdp4vs.dll`, and the RDP server/display stack.
  `rdp4vs.dll` creates its RDP server-base implementation before an RDP-base fallback. The exact
  object-level handoff from this host pipe to the guest listener remains private.

The cross-VM desktop limit is now attributed to a separate host LSM container-session gate. During
guest session arbitration, `lsm.dll` calls the parent over HVSock service
`{F58797F6-C9F3-4D63-9BD4-E52AC020E586}`. Host
`ContainerSessionServer::IncreaseTotalSessionCount` admits only one total container session on the
tested client host while the WVD policy is disabled. A later request returns Win32 error `353`
(`ERROR_MAX_SESSIONS_REACHED`), which the guest maps to
`ERROR_REMOTE_SESSION_LIMIT_EXCEEDED`; the denied guest session is then logged off. A guest event
capture observed the corresponding reason `12` and RDP Set Error Info `0x0C` about 32 seconds after
both clients initially reported connected.

The earlier wire `0x11` remains a real RDPIDD/IddCx failure path, but it can be a teardown symptom
after the same admission denial. Disabling vGPU did not remove the 32-second second-session logoff,
which rules out mirrored GPU assignment as the cross-VM root cause. Guest usernames, RDP clients,
display size, redirected channels, and per-VM Display Brokers are also excluded.

The static alternate branch is expected to work on **Windows 10 or Windows 11 Enterprise
multi-session under Azure Virtual Desktop**, where WVD multi-session policy is legitimately enabled.
Microsoft documents Windows Sandbox as supported in AVD desktop and RemoteApp sessions when the
session-host VM supports nested virtualization. This environment has not yet been exercised by the
IronRDP harness; it is a high-confidence prediction, not an observed result.
See [RDP and RDV transport](windows-sandbox-rdp-transport.md) and
[guest account identity](windows-sandbox-direct-lifecycle.md#guest-account-identity).

## Repository integration

- `ironrdp-agent` retains the product-supported `WindowsSandboxServer.exe` gRPC route and can
  connect to a known Sandbox named pipe.
- `ironrdp-wsb` holds the experimental direct-lifecycle wrapper. It deliberately does not claim to
  reimplement the privileged Windows Sandbox backend or the licensing policy.
- The external analysis artifacts, symbol databases, traces, and temporary UDK harness remain
  outside the repository. This directory contains only reproducible conclusions and references to
  committed IronRDP code.
