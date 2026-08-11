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

In particular, the notes explain the retail behavior that admits multiple running Sandbox VMs but
only one simultaneous full desktop in the tested configuration. That is an entitlement result
reported by Windows, not an IronRDP setting or a supported tuning surface.

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

    VM --> RDV[vmicrdv.dll + vmrdvcore.dll<br/>RDV VDEV and generic endpoint transport]
    RDV <--> GUEST[Sandbox guest]
    GUEST --> TERMSRV[termsrv.dll]
    TERMSRV --> LIC[tssrvlic.dll + LSCSHostPolicy.dll]
    LIC <--> RDV

    VM --> SYNTH[vmuidevices.dll + rdp4vs.dll<br/>Enhanced Mode VDEVs]
```

There are two distinct RDP-related paths in the host:

- The **direct Sandbox path** uses the guest Terminal Services listener over its type-2 VMBus
  transport. Its specialized acceptor creates the same guest RDP connection object used by the
  ordinary listener, then reaches guest licensing and the private LSCS/RDV bridge.
- The **Enhanced Mode path** is hosted by `vmuidevices.dll` and `rdp4vs.dll`. It has its own
  feature and ACL checks, VMBus/HVSock channels, and graphics pipeline. It is not the LSCS
  licensing receiver.

## Repository integration

- `ironrdp-agent` retains the product-supported `WindowsSandboxServer.exe` gRPC route and can
  connect to a known Sandbox named pipe.
- `ironrdp-wsb` holds the experimental direct-lifecycle wrapper. It deliberately does not claim to
  reimplement the privileged Windows Sandbox backend or the licensing policy.
- The external analysis artifacts, symbol databases, traces, and temporary UDK harness remain
  outside the repository. This directory contains only reproducible conclusions and references to
  committed IronRDP code.
