# Windows Sandbox architecture

This note maps the components involved when Windows Sandbox is started and an RDP desktop becomes
available. It is intentionally a relationship map: private boundaries are named where verified and
left unresolved where no evidence identifies their implementation.

See [direct VM lifecycle](windows-sandbox-direct-lifecycle.md),
[RDP and RDV transport](windows-sandbox-rdp-transport.md), and
[licensing and desktop admission](windows-sandbox-licensing.md) for focused details.

## Execution planes

Windows Sandbox spans five planes. A component can participate in more than one plane, but the
separation is useful when attributing an observed failure.

| Plane | Primary responsibility | Key components |
| --- | --- | --- |
| Product orchestration | Accept Sandbox configuration, create a product VM, produce connection data | Windows Sandbox launcher and `WindowsSandboxServer.exe` |
| VM lifecycle | Instantiate, hold, inspect, and terminate a private Sandbox VM | `ManagedWindowsVM.exe`, `WindowsUdk.Security.Isolation.ManagedWindowsVM`, private Container Manager APIs |
| Hyper-V worker | Host a particular VM and load its configured virtual devices | `vmcompute.exe`, `vmwp.exe`, registered VDEVs |
| Desktop transport | Carry RDP-related control/data traffic and render/input state | worker-owned VM-ID pipe, `vmuidevices.dll`, `rdp4vs.dll`, guest `termsrv.dll`, `rdpcorets.dll`, RDV VDEV |
| Desktop admission | Select a guest license library and request host policy | guest `tssrvlic.dll`, `LSCSHostPolicy.dll`, private RDV/RIM endpoint |

## Component relationship map

```mermaid
flowchart TB
    USER[Caller or IronRDP agent]

    subgraph product["Windows Sandbox product layer"]
        WSS[WindowsSandboxServer.exe]
        PRODUCTVM[Product-private VM provisioning]
    end

    subgraph lifecycle["Private VM lifecycle layer"]
        UDK[ManagedWindowsVM WinRT API]
        MVM[ManagedWindowsVM.exe]
        CMS[Private Container Manager]
    end

    subgraph host["Hyper-V host"]
        COMPUTE[vmcompute.exe]
        WORKER[vmwp.exe]
        PIPE[\\.\pipe\{VM-ID}]
        SYNTH[vmuidevices.dll / rdp4vs.dll / RDPSERVERBASE.dll]
        RDV[vmicrdv.dll]
        RDVCORE[vmrdvcore.dll / rdvvmtransport.dll]
    end

    subgraph guest["Sandbox guest"]
        TERMSRV[termsrv.dll]
        RDPCORETS[rdpcorets.dll]
        VMBUSPIPE[vmbuspipe.dll]
        TSSRVLIC[tssrvlic.dll]
        LSCS[LSCSHostPolicy.dll]
    end

    USER --> WSS --> PRODUCTVM
    USER --> UDK
    PRODUCTVM --> COMPUTE
    UDK --> MVM --> CMS --> COMPUTE --> WORKER
    WORKER --> PIPE
    WORKER --> SYNTH
    SYNTH --> PIPE
    RDV --> RDVCORE
    TERMSRV --> RDPCORETS
    TERMSRV --> TSSRVLIC --> LSCS
    LSCS <--> RDVCORE
```

The arrows show ownership or communication relationships observed in the tested build. They do not
imply a public API, a stable ABI, or a complete call graph.

For an exhaustive process, module, registration, and interface table, see
[the component and registration catalog](windows-sandbox-component-catalog.md).

## Host components

| Component | Observed role | Licensing relevance |
| --- | --- | --- |
| `WindowsSandboxServer.exe` | Product gRPC/orchestration service on a per-user `\\.\pipe\wsandbox\&lt;MD5(user SID)&gt;` pipe | Can reject VM admission before a VM exists; it is not the final LSCS host-policy implementation |
| `ManagedWindowsVM.exe` | Registered out-of-process WinRT implementation for `ManagedWindowsVM` | Provides private lifecycle orchestration; it is not a replacement for the privileged container backend |
| `vmcompute.exe` | Compute service that builds VM configuration and starts worker processes | Parses an internal `VirtualMachine/Devices/Licensing` resource and forwards opaque configuration to the worker |
| `vmwp.exe` | Per-VM worker process that hosts configured VDEVs and owns `\\.\pipe\{VM-ID}` | Does not statically contain the LSCS endpoint or `ILSClientService` markers; it hosts the observed RDP/display bridge |
| `vmicrdv.dll` | Registered `Msvm_RdvComponent` VDEV | Loads/uses generic RDV endpoint infrastructure |
| `vmrdvcore.dll` and `rdvvmtransport.dll` | Generic RDV/VMBus endpoint transport | Create endpoints from caller-supplied properties and dispatch payloads to a supplied sink; no concrete LSCS policy logic was found |
| `vmuidevices.dll` | Hyper-V Synthetic RDP and RDP Encoder VDEV implementation | Worker RDP/display bridge with named-pipe listeners; no LSCS marker |
| `rdp4vs.dll` | RDP4VS encoder engine loaded by `vmuidevices.dll` | Receives worker RDP pipes; no LSCS/RDV licensing markers |
| `RDPSERVERBASE.dll` | RDP server base loaded by the worker RDP4VS stack | Implements standard RDP wire licensing and display startup, but not the LSCS endpoint |

### RDV VDEV registration

The Hyper-V virtual-device registration maps the RDV VDEV class ID
`{6C5ADDB9-A11A-4E8E-84CB-E6208201DB63}` to the Microsoft RDV component. Its WMI setting class is
`Msvm_RdvComponentSettingData`; that registration explains how the worker loads the RDV VDEV for a
VM but does not reveal the application-level LSCS policy receiver.

## Guest components

| Component | Observed role |
| --- | --- |
| `termsrv.dll` | Starts the hard-coded type-2 VM listener, chooses the built-in licensing path, and owns the guest Terminal Services connection |
| `rdpcorets.dll` | Registered `UMRDPProtocolManager`; implements the VMBus acceptor and the shared `CUMRDPConnection` RDP server path |
| `vmbuspipe.dll` | Generic VMBus-pipe control/data channel dependency dynamically loaded by the guest VMBus listener |
| `tssrvlic.dll` | Activates the role-4 DVM proxy license library and its proxy policy |
| `LSCSHostPolicy.dll` | Guest bridge that invokes a private RDV/RIM endpoint and requests `IID_ILSClientService` from the host |

The guest components establish that desktop admission is not merely a host-side RDP listener count.
The detailed flow and its unresolved final host receiver are documented in
[licensing and desktop admission](windows-sandbox-licensing.md).

The listener implementation itself is documented in
[the guest RDP server](windows-sandbox-guest-rdp-server.md).

## Official and direct paths

The normal and direct paths share the same VM worker, guest image, Terminal Services endpoint, and
licensing architecture after the VM is running.

| Step | Product path | Experimental direct path |
| --- | --- | --- |
| Request lifecycle | Windows Sandbox launcher / `WindowsSandboxServer.exe` | `ManagedWindowsVM` through `ironrdp-wsb` |
| Product VM admission | Windows Sandbox server decides whether to create the VM | Bypassed only as an orchestration layer |
| Privileged backend | Product-private provisioning path; the exact lower call chain is not fully attributed | `ManagedWindowsVM.exe` and private Container Manager |
| Guest desktop listener | Windows Sandbox transport configuration | Same type-2 guest VMBus listener is statically present |
| Worker pipe bridge | Private product configuration | `vmwp.exe` owns the VM-ID pipe and loads the worker RDP/display bridge |
| Guest desktop admission | LSCS/RDV policy route | Same statically established LSCS/RDV policy route |

Direct lifecycle management therefore does not establish a new licensing identity or bypass desktop
admission. It changes who asks Windows to create the VM, not who implements the VM, the worker RDP
bridge, or the guest LSCS policy path.

## What was ruled out

The following were examined and are not supported as the final host-side `ILSClientService`
implementation in the tested configuration:

- `WindowsSandboxServer.exe` and `ManagedWindowsVM.exe`;
- guest `tssrvlic.dll` and the normal host `TermService` process;
- `vmicrdv.dll`, `vmrdvcore.dll`, `rdvvmtransport.dll`, `vmbuspipe.dll`, `vmbuspiper.dll`, and
  `vmprox.dll`;
- the static `vmwp.exe`, `vmcompute.exe`, `VmComputeAgent.exe`, and `vmcompute.dll` images;
- `vmuidevices.dll` and `rdp4vs.dll`.

This does not prove that all those processes are uninvolved. It establishes that the concrete
policy implementation is not statically attributable by the LSCS endpoint name, endpoint type,
or `ILSClientService` IID in those images.
