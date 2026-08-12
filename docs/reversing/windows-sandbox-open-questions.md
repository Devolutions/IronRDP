# Windows Sandbox unresolved questions and evidence plan

This note tracks what remains unresolved after the static analysis and controlled direct-Sandbox
tests. It prevents inference from being promoted to fact and keeps future investigation within a
read-only, no-bypass boundary.

## Current conclusions that do not need more attribution

| Conclusion | Evidence status |
| --- | --- |
| Direct `ManagedWindowsVM` lifecycle can create multiple running Sandbox VMs | Observed repeatedly |
| The product server can reject another Sandbox before VM creation | Observed and statically attributed to product orchestration |
| Multiple VMs may share the default guest username or use distinct custom users | Observed and statically attributed to product `SetUpUserAccount` / direct harness provisioning |
| Same-versus-different guest usernames is not the proven cross-VM concurrency root cause | Observed under both account patterns |
| The direct guest endpoint is a type-2 VMBus listener | Static guest `termsrv.dll` control flow |
| VMBus and TCP accepts converge on `CUMRDPConnection` | Static `rdpcorets.dll` control flow |
| The guest uses built-in, role-4 DVM proxy licensing before requesting host policy | Static guest `termsrv.dll`, `tssrvlic.dll`, and `LSCSHostPolicy.dll` control flow |
| Guest LSCS admission boundary | `CProxyPolicy` calls `ILSClientService::LSCSUserAuthenticated` over RIM |
| Earlier concurrent desktop rejection | Observed as `ERROR_REMOTE_SESSION_LIMIT_EXCEEDED` |
| Current concurrent desktop outcomes | All current cross-VM pairs reach `Connected`; later outcomes include `CloseStackOnDriverFailure` and server-reported `ERRINFO_LOGOFF_BY_USER`. Two clients to one VM are separately replaced by the worker-local RDP encoder path |

## Open attribution questions

| Question | Why it remains open | Narrow evidence needed |
| --- | --- | --- |
| Which runtime host component is the final RIM peer for `ILSClientService`? | The decision RPC is now identified as `ILSClientService::LSCSUserAuthenticated`. A target-worker capture found the VM-ID pipe and RDP/display bridge but no LSCS/RDV provider; a broader worker inventory found generic `vmicrdv`/`vmrdvcore` co-residence without an LSCS marker. Generic RDV and VMBus layers remain transport only. The standalone `TermService`/`vmicrdv` service path, HCS Licensing, lifecycle processes, and SPP activation are excluded | Read-only call-stack or RIM-object-registration capture at `LSCSUserAuthenticated`, correlated with the historical session-limit response, display-driver failure, and post-connect logoff |
| Is a current multi-desktop outcome an LSCS decision? | Not established. Both current cross-VM sessions reach `Connected` before either `CloseStackOnDriverFailure` or `ERRINFO_LOGOFF_BY_USER`; the worker has independent VM-view/input ACL and RDP-server lifecycle gates. The same-VM `ERRINFO_DISCONNECTED_BY_OTHERCONNECTION` result is separately proven to be the encoder's local replacement path | Correlate a call-stack or RIM-object-registration capture with the exact cross-VM error PDU; separately compare a Microsoft-supported alternate graphics setup without changing licensing or entitlement state |
| Which VDEV listener accepts the direct VM-ID pipe? | Both `RdpEncoder` and `SynthRdpDevice` can receive independently created pipe handles from `vmcompute` through the VDEV handle broker. RDP4VS exposes distinct pipe operations: the encoder's accept path creates an RDP connection, while SynthRDP asks the encoder for an `IRDPENCNetStream` for its VMBus/HVSock data channel. This makes the encoder the leading static candidate for the external endpoint, but HCS properties and ordinary Container/Hyper-V stores do not expose the selected connection-options record | Read-only accepted-pipe call stack or kernel file-I/O stack in the target `vmwp.exe` worker |
| Does the known SPP VM-pipe handler own desktop admission? | No. `sppsvc` processes the distinct Activation VDEV inherited-activation request/response and remained stopped throughout the tested direct desktop connection | No further SPP activation analysis is needed unless a future build shows it active during the LSCS flow |
| Is the receiver a conventionally registered VDEV? | The `vmwp.exe` manifest loader reads the `VirtualDevices` class/interface catalog, but the full catalog contains none of the LSCS interface, endpoint-instance, or endpoint-type identifiers. A direct Sandbox VM is not visible through the ordinary Hyper-V WMI system/settings associations | Inspect a live VM-specific repository and protected-worker registrations; do not infer a VDEV provider from the generic manifest mechanism |
| Where is the final entitlement state evaluated? | Guest identifies the interface boundary but not the host receiver's internal data source | Read-only call-stack or endpoint telemetry tied to an accepted and rejected admission |
| How do exact guest and host RDP component revisions align after servicing? | The supplied guest `termsrv.dll` and locally examined host modules are nearby but not identical revisions | Capture the guest `rdpcorets.dll`, `rdpbase.dll`, and VMBus-pipe versions from the same running Sandbox image |
| What guarantees does the product gRPC server add beyond direct lifecycle? | Product orchestration was identified, but full provisioning parity was not reconstructed | Compare read-only lifecycle/configuration traces for one product-created and one direct VM |
| Which private transport fields are stable across releases? | The SynthRDP handshake is not a published ABI | Re-run static listener analysis after component-version changes; do not assume compatibility |

## Evidence that would change the model

The following findings would materially revise the current documentation:

1. A concrete runtime host module that receives the RIM request and owns the admission decision,
   rather than merely offering or transporting its VMBus channel.
2. A complete same-build guest capture showing a different `rdpcorets.dll` listener implementation.
3. A repeatable Windows-supported entitlement environment with a different admission outcome.
4. Evidence that the direct named-pipe route reaches a different guest licensing selector.

Absent that evidence, the final LSCS receiver must remain described as **host-mediated and
dynamically unattributed**.

## Safe investigation boundaries

Future work may:

- inspect static binaries, version metadata, registrations, and exported interfaces;
- compare module/image-load telemetry before and after a Sandbox desktop connection;
- create and terminate only test VMs owned by the investigation harness;
- compare observed protocol errors and ordinary RDP traces; and
- document differences between Windows versions or Microsoft-defined environments.

Future work must not:

- modify licensing, entitlement, session-count, or virtualization policy state;
- patch, inject into, or replace protected Windows components;
- treat private listener identifiers as a supported external ABI; or
- claim that an RDP client option changes the host desktop-admission decision.

## Effect on IronRDP scope

The unresolved host policy receiver does not block the supported engineering boundary:

| Capability | Status |
| --- | --- |
| Product-server Sandbox lifecycle | Implemented in `ironrdp-agent` |
| Experimental direct lifecycle wrapper | Implemented in `ironrdp-wsb` with explicit private-API limitations |
| Standard RDP client over a Windows-provided Sandbox pipe | Implemented by the ordinary IronRDP RDP stack |
| Reimplementing Windows VM engine, listener, or licensing provider | Out of scope |
| Claiming multi-desktop entitlement on retail Windows | Unsupported by the observed evidence |

See [licensing and desktop admission](windows-sandbox-licensing.md) for the decision tree and
[the evidence matrix](windows-sandbox-evidence.md) for the current component/version scope.
