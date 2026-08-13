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
| `vmwp.exe` | `10.0.26100.8457` in earlier analysis and `10.0.26100.8875` in the later local copy | Per-VM worker; creates manifest-selected VDEVs with `CoCreateInstance(CLSID, IID_IVirtualDevice)` but has no static LSCS endpoint or `ILSClientService` marker |
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
| `vmuidevices.dll` | `10.0.26100.8457` | COM-loaded Synthetic RDP and RDP Encoder VDEVs; explicitly loads `rdp4vs.dll` for its pipe listener and RDP server instances |
| `rdp4vs.dll` | `10.0.26100.5074` | RDP4VS worker engine; exports `RDP4VS_CreateInstance` for the named-pipe listener and main server, then selects `RDPSERVERBASE_CreateInstance` before its `RDPBASE` fallback |
| `VrdUmed.dll` | `10.0.26100.1150` | Virtual Render Device user-mode emulation driver used by the worker GPU-partition path |
| `GpupVDev.dll` | `10.0.26100.8875` | Hyper-V GPU-partition VDEV that creates the worker's UMED provider |
| `windowsudk.winmd` | `10.0.26100.1` | Metadata source for the checked-in narrow UDK projection |

Windows component servicing can replace individual binaries. Conclusions should be revalidated when
these versions change materially. The five-hypothesis runtime matrix was repeated on a host reporting
Windows `10.0.26200`; its installed `rdpbase`, `rdp4vs`, `RDPSERVERBASE`, `vmuidevices`, `gpupvdev`,
and `VrdUmed` files still had the `10.0.26100` versions listed above.

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

An elevated whole-worker module inventory also found ordinary `vmwp.exe` workers that load
`vmuidevices.dll`, `vmicrdv.dll`, and `vmrdvcore.dll` together. Those worker IDs could not be
correlated to the direct-Sandbox test artifacts, so the observation only shows that the generic RDV
VDEV can co-reside with the RDP/display VDEVs. It does not identify it as the target worker's LSCS
receiver or contradict the target-specific module snapshot that lacked those modules.

The two observed RDP VDEV CLSIDs are normal free-threaded in-proc COM registrations whose
`InprocServer32` path is `C:\Windows\System32\vmuidevices.dll`:

| VDEV | CLSID | COM ProgID | VDEV catalog interface |
| --- | --- | --- | --- |
| RDP Encoder | `{9CB98DB1-4D09-4538-A192-2D3D8C0B6CDB}` | `RdpEncoderVdev.1` | `{1F490E46-8AD0-468D-8359-BA9D57FE9CC8}` |
| Synthetic RDP Device | `{9ED5FD4B-40C3-4DE3-8597-98ECD17035DA}` | `SynthRdpVdev.1` | None declared in the examined catalog entry |

The encoder exposes the same private catalog interface from `RdpEncoder::InnerQueryInterface`;
`SynthRdpDevice` declares an `IRdpEncoder` service dependency. The GUID is not registered under
`HKCR\Interface`, so this remains a private VDEV contract rather than a normally activatable COM
interface.

At power-on, a configured RDP Encoder named-pipe path creates a `NamedPipeListenerBinding`. It
loads `rdp4vs.dll`, resolves `RDP4VS_CreateInstance`, and requests
`CLSID_RDP4VPCNamedPipeListener` as `IID_IRDP4VPCNamedPipeListener`. On pipe acceptance, the
encoder separately requests `CLSID_RDP4VS` as `IID_IRDP4VS`, queries
`IID_IRDP4VSNamedPipe`, and passes the accepted handle through that private interface. The
corresponding callback path supplies an `IAfSecurityInfo` token to the encoder, which creates an
ACL context through `ISecurityManager` and checks `AuthFAccessObjectTypeVirtualMachine`.

`vmcompute.exe`'s VDEV dependency map establishes that the Video Monitor requires the RDP Encoder
and Input Manager, while SynthRDP requires the RDP Encoder and Guest Interface. Its separate
`ConfigureRdpEncoder` and `ConfigureSynthRdp` routines can each create a server pipe from their
connection-options record. The shared creation routine applies configured access SIDs, the VM SID,
and a fixed RDP capability SID to the DACL before transferring the handle to the VDEV through the
handle broker. This is direct static evidence for the compute-service origin of the pipe handle,
but not for which per-device configuration receives the direct VM-ID pipe name.

Further RDP4VS static analysis resolves the two private named-pipe call forms: the encoder's
accepted-pipe callback invokes the operation that creates an RDP connection, while SynthRDP's
encoder callback invokes the operation that creates an `IRDPENCNetStream` for the Synthetic RDP
data channel. This is strong architectural evidence that the encoder owns the external RDP
connection and SynthRDP bridges its stream to the guest. It is not final listener attribution
without a target-worker accepted-pipe stack.

A disposable direct Sandbox probe opened its HCS system with the documented read-only
`HcsGetComputeSystemProperties` path. It returned only basic runtime state, owner
`WindowsSandbox`, and the `CmService` template runtime ID. No configuration properties were
available, and no VM-ID-bearing configuration file was found under the known Container or Hyper-V
stores. The HCS property API and ordinary storage inventory cannot resolve the listener selection.

Public `vmcompute.exe` symbols show that the HCS `VirtualMachine/Devices/Licensing` resource is
not an LSCS configuration path. `ModifyLicensingSettings` obtains a handle broker for
`ACTIVATION_INSTANCE_ID` and calls `IVmVirtualDeviceAccess`; the default VDEV manifest adds
`ACTIVATION_DEVICE_ID` for the same setting. This is the `ActivationVDev.dll` path.

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

The authenticated admission call is explicit. Guest `CProxyPolicy::UserAuthenticated` invokes
`IHostPolicy::HostUserAuthenticated`, and `CHostPolicy::HostUserAuthenticated` obtains the RIM
proxy then calls `ILSClientService::LSCSUserAuthenticated`. The generated
`CStubILSClientService::Invoke_LSCSUserAuthenticated` unmarshals the protocol context and user
strings before dispatching to the concrete host object. This is the direct host decision boundary
for the observed session-limit response.

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
| Default guest account multi-VM setup | Three concurrent VMs each used guest username `WDAGUtilityAccount` with distinct per-VM passwords |
| Custom guest account multi-VM setup | Three concurrent VMs used distinct guest usernames `IrdpVm1`, `IrdpVm2`, and `IrdpVm3` |
| Per-VM RDP client identity | Distinct GCC client names were used for separate VMs |
| First direct desktop | Connected successfully |
| Historical later concurrent desktop | Failed with `ERROR_REMOTE_SESSION_LIMIT_EXCEEDED` |
| Official Sandbox server path | Can reject another active Sandbox before it creates a VM |
| Medium-integrity worker inspection | Insufficient for reliable protected-worker module and kernel image-load attribution |
| First elevated trace | Captured worker initialization/vPCI activity, but no LSCS payload receiver or post-connect module delta |
| Direct connected-VM service probe | `TermService` and `vmicrdv` both remained stopped throughout a successful IronRDP desktop connection |
| Direct connected-worker module probe | The new protected `vmwp.exe` worker could not be enumerated at medium integrity; `tasklist /m` did not attribute candidate RDV modules |
| Direct VM WMI-manifest probe | Returned VM ID did not enumerate as `Msvm_ComputerSystem`, so ordinary Hyper-V WMI could not expose VDEV settings |
| Direct desktop SPP-service probe | Across 174 samples, `sppsvc` and `ClipSVC` remained stopped; `LicenseManager` remained running and no `sppsvc.exe` process appeared |
| Lifecycle-module differential | `ManagedWindowsVM.exe` and the per-VM `cmproxyd.exe` loaded no module between VM creation and successful LSCS authentication |
| Temporal pipe differential | No private LSCS candidate appeared in the named-pipe namespace during the authentication window |
| Temporal process differential | No licensing/RDV helper process appeared; the repeatable new `svchost.exe` hosted `FrameServerMonitor` only |
| Elevated VM-ID pipe capture | `\\.\pipe\{VM-ID}` is owned by the target Sandbox `vmwp.exe` worker; no process loaded `LSCSHostPolicy.dll` |
| Elevated worker module capture | Connection-time worker stack includes `vmuidevices`, `rdp4vs`, `RDPBASE`, `RDPSERVERBASE`, `gpupvdev`, and `VrdUmed`; it excludes `LSCSHostPolicy`, `vmicrdv`, `vmrdvcore`, and `rdvvmtransport` |
| Two-VM direct desktop comparison | Both sessions first reach `Connected`; the second later sends `CloseStackOnDriverFailure` (`0x11`) while the first remains connected |
| Resolution-only two-VM comparison | Both sessions first reach `Connected`; the first later sends `CloseStackOnDriverFailure` and the second later reports `ERRINFO_LOGOFF_BY_USER` |
| Timestamped same-size baseline | Both sessions first reach `Connected`; 33 seconds later only the second reports `ERRINFO_LOGOFF_BY_USER`, while the first remains connected |
| Same-VM two-client control | The first session is immediately disconnected with `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` when the second session connects to the same Sandbox VM; the second remains connected |
| HCS GPU configuration | Both test VMs use `VirtualMachine/ComputeTopology/Gpu` with `AssignmentMode: Mirror` and `AllowVendorExtension: true` |
| Host analytic channels | Worker and Worker-VDev Analytic channels were enabled only for the bounded comparison, produced no matching RDP/display events, and were restored to disabled |
| Microsoft ActiveX cross-VM control | Two MSTSC ActiveX controls using the Microsoft `RDPBASE` named-pipe connector both reached `Connected`; the first disconnected before its timer while the second remained connected until its intentional timer close |
| Reverse-order and long-stagger controls | Reversing connection order did not eliminate the failure; a prior roughly three-minute stagger between connected desktops also failed |
| Direct-guest TCP control availability | TCP port 3389 was closed on both direct-VM NAT addresses, so the default path offered no policy-preserving TCP comparison |
| Minimal display control | At `640x480` and 16 bpp, both desktops connected before one received the display-driver error and the other later logged off |
| Minimal channel control | Disabling CLIPRDR and RDPSND did not change the display-driver-then-logoff sequence |
| Host event correlation | MSTSC ActiveX client events recorded the early disconnect separately from the later intentional timer close; no correlated host `DxgKrnl` or worker operational error identified the server-side owner |

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
| HCS `VirtualMachine/Devices/Licensing` controls LSCS admission | Ruled out | Public `vmcompute` symbols route it exclusively to `ACTIVATION_INSTANCE_ID` through `IVmVirtualDeviceAccess` |
| VMBus pipe/proxy layers own LSCS policy | Ruled out | Public `vmbuspipe` and `vmbusproxy` symbols implement generic channel operations only |
| Standard host RDP wire licensing is the LSCS admission decision | Strongly disfavored | `RDPSERVERBASE!IsLicenseRequiredByOS` skips standard licensing on non-server Windows |
| Current direct two-VM disconnect is an LSCS denial | Not established | Current post-connect outcomes are `CloseStackOnDriverFailure` and `ERRINFO_LOGOFF_BY_USER`; neither run includes a correlated LSCS call trace |
| Wire `CloseStackOnDriverFailure` (`0x11`) is named as an Indirect Display Driver failure inside RDPBASE | High for naming, inference for live cause | `GetInternalDisconnectSymbolicName(17)` returns `IndirectDisplayDriverFailure`; adjacent codes cover IDD not-ready and interface-arrival failures |
| Current `0x11` is every GFX-pipe `SetPipelineErrorState` failure | Ruled out | `CPipeManager::SetPipelineErrorState` maps subsystem-init and related pipe events to reasons such as `4460`/`4461`, not wire `0x11` |
| IronRDP causes the current cross-VM failure | Ruled out | The Microsoft MSTSC ActiveX/RDPBASE path reproduces an early cross-VM disconnect |
| CLIPRDR or RDPSND startup causes the current cross-VM failure | Ruled out | The failure sequence remains with both channels disabled |
| The current IDD failure is a proportional framebuffer-size limit | Strongly disfavored | `640x480` at 16 bpp and the earlier `800x600` control did not prevent it |
| The current failure is only a short VM boot or first-logon race | Strongly disfavored | Reverse-order and roughly three-minute stagger controls still fail |
| The default direct path can be compared over guest TCP RDP | Unavailable | Neither direct VM exposed TCP port 3389; enabling it would change guest policy and was not attempted |
| A host-global IDD/presentation ownership or lifecycle conflict causes current failures | Leading inference | Separate workers and clients still collide; wire `0x11` is named `IndirectDisplayDriverFailure`; reducing display load does not help |
| Guest usernames must match across concurrent Sandboxes | Ruled out | Product and direct harnesses can use either the shared default name or distinct custom accounts; both patterns still produce multi-VM runs and current post-connect failures |
| Concurrent VMs share one guest user object | Ruled out | Each VM has its own guest SAM; product `SetUpUserAccount` activates or creates a local account inside that guest only |
| Worker RDP path has an independent access gate | High | `RdpEncoder::OnClientConnected` creates an ACL policy engine before accepting the RDP4VS attendee |
| Worker RDP path hosts an RDP server stack | High | `rdp4vs!RDPAPI_CreateInstance` selects `RDPSERVERBASE_CreateInstance`, with `RDPBASE` only as fallback |
| Sandbox has an independent guest RDP server | Ruled out | VMBus and TCP listener paths both create `CUMRDPConnection` in `rdpcorets.dll` |
| Exact final host `ILSClientService` implementation | Unresolved, narrowed | The only direct decision call is `LSCSUserAuthenticated`; elevated worker capture found no LSCS client or RDV module, leaving a runtime registration boundary |

## Completed elevated capture and next evidence

The focused elevated capture completed with one direct Sandbox VM and a successful authenticated
desktop. It recorded kernel image loads and before/after worker module lists. The worker owns the
VM-ID pipe and loaded the RDP/display bridge, but did not load an LSCS/RDV provider. No relevant
post-authentication image load identified the final RIM object.

The next narrow read-only step is a call-stack or RIM-object-registration capture at
`ILSClientService::LSCSUserAuthenticated`, compared across the historical session-limit response,
the display-driver failure, and the post-connect logoff. This investigation does not alter policy,
registry state, license data, session limits, host binaries, or Sandbox behavior.

The outstanding questions, evidence threshold for each, and investigation guardrails are maintained
in [open questions and evidence plan](windows-sandbox-open-questions.md).
