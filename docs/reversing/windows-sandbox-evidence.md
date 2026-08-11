# Windows Sandbox evidence and version matrix

These notes are based on read-only static analysis, controlled direct-VM tests, and limited host
tracing. The source binaries and generated analysis databases are intentionally not committed.

## Evidence classes

| Label | Meaning |
| --- | --- |
| Observed | Reproduced on the tested host or guest during a controlled test |
| Static | Recovered from an examined binary's code, symbols, registrations, imports, or data |
| Inference | Best explanation consistent with observed/static evidence; not presented as a product contract |
| Unresolved | Requires more evidence before assigning a component or a behavior |

The other notes use these classes implicitly. When in doubt, the wording "observed" or "static
analysis" is stronger than "likely" or "suggests".

## Tested component family

| Component | Version examined | Finding |
| --- | --- | --- |
| `vmwp.exe` | `10.0.26100.8457` in earlier analysis and `10.0.26100.8875` in the later local copy | Per-VM worker; no static LSCS endpoint or `ILSClientService` marker |
| `vmcompute.exe` | `10.0.26100.8457` | Parses internal VM licensing resource and starts/controls workers |
| `VmComputeAgent.exe` | `10.0.26100.1` | No static LSCS endpoint or `ILSClientService` marker |
| `vmicrdv.dll` | `10.0.26100.1` | Registered RDV VDEV; initializes only its generic `RdvVmEndPointTransport` endpoint |
| `vmrdvcore.dll` | `10.0.26100.1150` | Generic RDV endpoint/VMBus transport; its offer selector recognizes the LSCS instance but contains no `ILSClientService` policy implementation |
| `rdvvmtransport.dll` | `10.0.26100.1` | Standalone generic RDV transport with the same LSCS offer selector and no `ILSClientService` marker |
| `ActivationVDev.dll` | `10.0.26100.8737` | Dynamically selected SPP activation VDEV; inherited activation and a separate fixed VMBus-pipe service |
| `LSCSHostPolicy.dll` | `10.0.26100.7623` | Guest LSCS/RIM bridge |
| `vmbusvdev.dll` | `10.0.26100.8457` | Generic worker VMBus VDEV; exposes `IVMBusTransport`, not LSCS policy |
| Guest `termsrv.dll` | `10.0.26100.8115` | Selects built-in licensing for listener types 2 and 3 |
| Guest `tssrvlic.dll` | `10.0.26100.8655` | Role-4 DVM proxy licensing route |
| `rdpcorets.dll` | `10.0.26100.8737` | Implements `UMRDPProtocolManager`, the VMBus listener, and the shared RDP connection object |
| `rdpbase.dll` | `10.0.26100.8875` | Shared RDP base factory and platform infrastructure used by `rdpcorets.dll` |
| `vmbuspipe.dll` | `10.0.26100.8521` | Exact VMBus listener dependency, including channel notification APIs |
| `vmbuspiper.dll` | `10.0.26100.8521` | Related generic VMBus-pipe component, not the notification DLL selected by `rdpcorets.dll` |
| `vmuidevices.dll` | `10.0.26100.8457` | Synthetic RDP and RDP Encoder VDEVs |
| `rdp4vs.dll` | `10.0.26100.5074` | RDP4VS encoder engine used by Enhanced Mode |
| `windowsudk.winmd` | `10.0.26100.1` | Metadata source for the checked-in narrow UDK projection |

Windows component servicing can replace individual binaries. Conclusions should be revalidated when
these versions change materially.

## Static-analysis method

The investigation used:

- decompilation and control-flow analysis of the relevant DLLs and executables;
- raw GUID/IID byte searches across selected host binaries;
- virtual-device registration and WMI metadata inspection;
- import/string analysis to distinguish generic endpoint transports from policy code; and
- source-level inspection of the supplied Windows Sandbox managed binaries and the public
  WindowsSandboxPlayground reversing notes.

The public notes are available at
[gerneio/WindowsSandboxPlayground](https://github.com/gerneio/WindowsSandboxPlayground). They
provided useful names and hypotheses, but conclusions in this directory were corroborated against
the locally examined binaries or controlled runtime behavior.

An important false positive was `aitstatic.exe`. It contains both the LSCS interface ID and endpoint
type as entries in writable, dense GUID catalogs used by the Application Impact Telemetry Static
Analyzer. It is not a VMBus, RDV, RIM, or LSCS policy service.

### Expanded host marker scan

An additional read-only raw-byte scan on the tested host searched for:

- the native little-endian layout of `IID_ILSClientService`;
- the LSCS endpoint type and instance GUIDs; and
- ASCII and UTF-16 forms of `RdvVmEndPointLSCS` and `ILSClientService`.

| Scope | Binaries scanned | Result |
| --- | ---: | --- |
| Top-level `C:\Windows\System32` DLLs, EXEs, and drivers | 4,296 | `LSCSHostPolicy.dll` and `aitstatic.exe` contain the interface ID; `vmrdvcore.dll` and `rdvvmtransport.dll` contain the LSCS endpoint type and instance, but not the interface ID |
| Host `VirtualDevices` VDEV registry catalog | All registered class/interface entries | No LSCS interface, endpoint-instance, or endpoint-type identifier |
| `C:\Windows\System32\drivers` | 464 | No marker |
| `C:\Windows\SystemApps` | 746 | No marker |
| Supplied Windows Sandbox package artifacts | 777 | No marker |

The scan also found no `HKCR\Interface` registration for
`{B3461E73-AE5B-465B-8BDB-42BC5E87F22C}`. This reinforces that the request is not a conventional
COM activation contract. It does not rule out a host provider registered dynamically through the
generic RDV/RIM layer, one that receives an already-marshaled request, or a component unavailable
to the unprivileged package inventory.

The host service executables and DLLs most plausibly adjacent to the path (`vmms.exe`,
`vmcompute.exe`, `vmcompute.dll`, `VmComputeProxy.dll`, `vmmsprox.dll`, `icsvcext.dll`,
`LicenseManagerSvc.dll`, and `LicenseManager.dll`) also contain none of the searched LSCS names or
interface markers. This is negative evidence only; it does not establish that those processes are
uninvolved at runtime.

### RDV dispatch and dynamic VDEV boundary

IDA analysis establishes two constraints that are more precise than the raw-marker scan:

1. `vmicrdv.dll` loads `vmrdvcore.dll`, creates the generic role-3 transport endpoint named
   `RdvVmEndPointTransport`, and forwards endpoint names and GUIDs supplied to its server RPC.
2. `vmrdvcore.dll` creates endpoints from caller-supplied properties and an optional
   `IRdvVmEndPointSink`. `vmicrdv.dll` passes null, which installs internal relay sinks that copy
   received payloads between the VMBus and named-pipe halves.

The shared `CRdvVmbusChannel::OfferChannel` code in both RDV transport binaries compares an
endpoint instance with the LSCS instance GUID and selects the LSCS endpoint type before calling
`VmbusPipeServerOfferChannel`. Thus the transport can offer LSCS when a caller provides those
properties. Its built-in role-1 setup creates only `RdvVmEndPointTransport` and unrelated generic
endpoints, not LSCS, and neither binary contains `IID_ILSClientService`.

The RDV DLLs therefore explain the generic offer/relay boundary but do not implement the LSCS
admission policy themselves. The endpoint-creation server returns a fresh token generated from 15
random bytes for its named-pipe side, so `RdvVmEndPointLSCS` is not a persistent pipe name available
for static inventory searches.

The worker is designed to load VDEVs dynamically: `vmwp.exe` populates a VDEV manifest from the VM
repository and instantiates each entry with `CoCreateInstance(CLSID, IID_IVirtualDevice)`. This
means a static lack of LSCS markers in `vmwp.exe` cannot exclude a manifest-selected provider.

The worker's `ReadVDevClassInfo` reads conventional class/interface data from
`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\VirtualDevices`. Searching every
entry in that catalog found none of the three LSCS identifiers. The result rules out a VDEV that
conventionally advertises LSCS, but not a dynamically configured worker handler or a provider with
no LSCS marker in its registration.

For an isolated direct Sandbox VM, its returned VM ID was not visible as an
`Msvm_ComputerSystem` in `root\virtualization\v2`. The ordinary WMI settings associations therefore
could not expose its VDEV manifest. This is a limitation of the direct-Sandbox visibility surface,
not evidence that the worker or its manifest does not exist.

### LSCS client direction

PDB-backed static analysis of `LSCSHostPolicy.dll` resolves
`CHostPolicy::ConnectToHostAndIntegrateWithRim` and `CHostPolicy::HostProcessData`.
`CChannelMgr::Connect` calls `CVmBusNamedPipeChannel::ConnectToParentPartition`; the resulting
RIM object manager then creates a proxy for `IID_ILSClientService` with object ID `1`.
`HostProcessData` forwards its request through that proxy.

This is stronger than an endpoint-string inference: the examined `CHostPolicy` instance is a
guest-to-parent client adapter, not the concrete host `ILSClientService` object. The shared
library also includes generic server-capable channel and RIM-stub code, so their presence alone
does not attribute a host implementation to this DLL.

`vmbusvdev.dll` was separately decompiled. It opens the worker's VMBus device and returns
`IVMBusTransport` to other VDEVs, but contains no LSCS/RIM endpoint or licensing-provider code.
It is transport plumbing, not the final receiver.

`ActivationVDev.dll` is one such provider and is the only directly identified VDEV adjacent to the
internal `Licensing` VM resource. Static analysis identifies inherited activation/Client Licensing
Platform behavior and a direct VMBus-pipe server using
`{4487B255-B88C-403F-BB51-D1F69CF17F87}` and
`{3375BAF4-9E15-4B30-B765-67ACB10D607B}`. Those values are distinct from both LSCS endpoint GUIDs,
and the DLL contains no LSCS/RDV markers or `IID_ILSClientService`. It is consequently strong
negative evidence against a direct LSCS-sink attribution, not proof that it owns the desktop policy.

`sppsvc.exe` is the host peer for the separate Activation VDEV. Its
`SLSProcessVMPipeMessage` handler consumes `SppIABindingVmId` and
`SppBindingActivationRequest`, invokes
`msft:spp/notifications/common/processvmpipemessage`, then returns
`SppBindingActivationResponse`. The guest-facing Activation VDEV contains the matching
`SLpProcessVMPipeMessage` and `InheritedActivationRequest` names. This connects the fixed
Activation VDEV channel to Software Protection Platform, not to LSCS.

A controlled successful direct desktop connection sampled `sppsvc`, `ClipSVC`, and
`LicenseManager` 174 times. `sppsvc` and `ClipSVC` remained stopped, while `LicenseManager`
remained running; no `sppsvc.exe` process was observed. This rules out the known SPP activation
handler as the active LSCS receiver for the tested direct flow.

## Dynamic observations

| Test | Result |
| --- | --- |
| Direct UDK VM creation | Multiple Sandbox VMs were created and remained running concurrently |
| Per-VM RDP client identity | Distinct GCC client names were used for separate VMs |
| First direct desktop | Connected successfully |
| Concurrent later desktop | Failed with `ERROR_REMOTE_SESSION_LIMIT_EXCEEDED` |
| Official Sandbox server path | Can reject another active Sandbox before it creates a VM |
| Medium-integrity worker inspection | Insufficient for reliable protected-worker module and kernel image-load attribution |
| First elevated trace | Captured worker initialization/vPCI activity, but no LSCS payload receiver or post-connect module delta |
| Direct connected-VM service probe | `TermService` and `vmicrdv` both remained stopped throughout a successful IronRDP desktop connection |
| Direct connected-worker module probe | The new protected `vmwp.exe` worker could not be enumerated at medium integrity; `tasklist /m` did not attribute candidate RDV modules |
| Direct VM WMI-manifest probe | Returned VM ID did not enumerate as `Msvm_ComputerSystem`, so ordinary Hyper-V WMI could not expose VDEV settings |
| Direct desktop SPP-service probe | Across 174 samples, `sppsvc` and `ClipSVC` remained stopped; `LicenseManager` remained running and no `sppsvc.exe` process appeared |

## Confidence boundaries

| Conclusion | Confidence | Basis |
| --- | --- | --- |
| `WindowsSandboxServer.exe` is orchestration/admission, not the VM engine | High | Managed server analysis plus direct UDK lifecycle success |
| Direct lifecycle still uses a privileged Windows backend | High | `ManagedWindowsVM.exe` and Container Manager relationship |
| The direct guest listener is type 2 and uses built-in licensing | High | Guest `termsrv.dll` control flow |
| The role-4 guest licensing path reaches `LSCSHostPolicy.dll` | High | Guest `tssrvlic.dll` control flow |
| `LSCSHostPolicy.dll` is a guest-to-parent RIM client adapter | High | `CHostPolicy` and `CChannelMgr::ConnectToParentPartition` control flow |
| Desktop admission is host mediated | High | Guest `ILSClientService` request plus repeated cross-VM result |
| Normal RDP session-count configuration controls the result | Ruled out | Configuration and control-flow analysis |
| `vmuidevices.dll` owns the LSCS decision | Ruled out | Dedicated Enhanced Mode control flow and absent LSCS markers |
| `ActivationVDev.dll` is the direct LSCS sink | Strongly disfavored | Separate fixed VMBus-pipe identifiers, inherited-activation CLIP behavior, and no LSCS/RDV markers |
| Host `TermService` or `vmicrdv` is the active direct-flow LSCS receiver | Ruled out for the tested flow | Both services remained stopped during a successful direct desktop connection |
| `vmrdvcore.dll` or `rdvvmtransport.dll` owns LSCS policy | Ruled out | LSCS-specific offer selector only; no `IID_ILSClientService` or policy sink implementation |
| A conventionally registered VDEV advertises LSCS | Ruled out | No LSCS marker in the full `VirtualDevices` class/interface catalog |
| The known `sppsvc` VM-pipe activation handler is the active LSCS receiver | Ruled out for the tested flow | Its explicit inherited-activation request/response differs from LSCS, and `sppsvc` remained stopped during direct desktop admission |
| Sandbox has an independent guest RDP server | Ruled out | VMBus and TCP listener paths both create `CUMRDPConnection` in `rdpcorets.dll` |
| Exact final host `ILSClientService` implementation | Unresolved | Generic RDV transport is identified; a concrete policy receiver is not |

## Pending read-only capture

The focused trace has two components:

1. an elevated collector that records kernel image-load events and RDV/worker telemetry, then
   snapshots the newly created direct-Sandbox `vmwp.exe` before and after a real desktop
   connection; and
2. a non-elevated runner that creates one direct Sandbox VM, connects an isolated IronRDP daemon,
   signals the collector, and cleans up only its own VM, harness, and daemon.

The scripts were syntax-validated after adding lifecycle cleanup. The remaining prerequisite is an
available Administrator session to run the collector through a complete post-connect interval.

This capture is for attribution only. It does not alter policy, registry state, license data,
session limits, host binaries, or Sandbox behavior.

The outstanding questions, evidence threshold for each, and investigation guardrails are maintained
in [open questions and evidence plan](windows-sandbox-open-questions.md).
