# IronRDP ActiveX

`ironrdp-activex` is a Windows in-process COM Automation server that provides a practical, explicit
foundation for hosting IronRDP behind classic MSTSCLib automation clients. It builds as a `cdylib` and
exports the standard COM server entry points:

- `DllGetClassObject`
- `DllCanUnloadNow`
- `DllRegisterServer`
- `DllUnregisterServer`
- `DllGetTscCtlVer`
- `DllSetAuthProperties`
- `DllGetClaimsToken`
- `DllSetClaimsToken`
- `DllLogoffClaimsToken`
- `DllCancelAuthentication`
- `DllDeleteSavedCreds`

Windows builds embed a standard `VERSIONINFO` resource. Explorer and Windows API version queries report
the Cargo package version as the file and product version, with `IronRDP ActiveX Control` as the file
description, `IronRDP ActiveX` as the product name, and `ironrdpax.dll` as the original filename.

The crate targets the public MSTSCLib Automation and windowed ActiveX-hosting contracts. It is
deliberately not presented as a drop-in binary replacement for Microsoft's `mstscax.dll`.
Unsupported Automation and raw-interface members return `DISP_E_MEMBERNOTFOUND` or `E_NOTIMPL`
rather than a false success result. A small set of mstsc-compatible status and lifecycle defaults is
provided where the published control accepts the operation without starting an unavailable feature.

## Local agent RPC

Set `IRONRDP_ACTIVEX_RPC=1` in the ActiveX host process before creating the control to expose the
current-user-only `ironrdp-agent` RPC service from `ironrdpax.dll`. The shared transport applies a
protected current-user DACL to its named pipe. Its default endpoint is
`ironrdp-activex-<user>` on Windows; set `IRONRDP_ACTIVEX_RPC_ENDPOINT` to select another named
pipe. The listener never accesses the STA control directly: connection, disconnect, input, resize,
and configuration requests are dispatched through the control's message-only window. `Shutdown`
stops only this listener after replying; it does not close the host or disconnect the RDP session.

## Registration

Register a bitness-matching build with:

```powershell
regsvr32 .\ironrdpax.dll
```

Registration writes only the IronRDP-owned class and ProgIDs:

- `CLSID\{5D3E2B4C-6860-462E-8E9D-0C4D2B094C5F}`
- `IronRDP.ActiveX`
- `IronRDP.ActiveX.1`

The class is registered with `ThreadingModel=Apartment`. It does not overwrite Microsoft MSTSCLib
registrations or register a synthetic type library. The public MSTSCLib library identifier is
`{8C11EFA1-92C3-11D1-BC1E-00C04FA31489}`, version `1.0`; this crate preserves its published DISPIDs
for the Automation members it implements without claiming to provide that typelib.

For direct DLL probing, such as a configured MsRdpEx `mstscax` DLL path, `DllGetClassObject` recognizes
the IronRDP CLSID and the published MSTSCLib client coclass aliases through v12. The v12 probe's
`IMsRdpClientNonScriptable7` contract is ABI-complete; only its cursor-scaling policy is currently mapped.

The auxiliary authentication and claims-token exports use the published mstscax ABI so MsRdpEx can
resolve them safely. They return `E_NOTIMPL` (and clear `DllGetClaimsToken` output BSTRs) because
IronRDP does not implement the proprietary claims-token service. MsRdpEx must use an explicit
IronRDP backend/capability gate to skip its undocumented `IMsTscAx` object-memory scan for the
Microsoft `CTSPropertySet`; this control intentionally never emulates that private layout. Native
`mstsc.exe` RemoteApp startup depends on that unavailable private Microsoft state and is therefore
unsupported with the IronRDP backend. Use the MsRdpEx/WinForms host route or another public
Automation host for IronRDP connections.

For bounded host-call diagnostics, set `IRONRDP_ACTIVEX_HOST_TRACE` to an output file path.
The resulting trace records only method/state markers and requested viewport dimensions, never
Automation property values, and must be enabled explicitly.
During local OLE-layout diagnosis, `Renderer::SetObjectRectsChanged`,
`Renderer::PaintRetainedFrameAfterLayout`, and `Renderer::FrameAcceptedAfterLayout` distinguish
an existing retained-frame repaint from the first newly accepted remote frame after a layout change.
Unsupported `IMsRdpClientAdvancedSettings` calls are recorded as
`E_NOTIMPL:AdvancedSettings::slot_<n>` so a host trace can be mapped directly to the documented
raw-vtable slot before a setting is implemented.

### Native mstsc credential bridge

The native mstsc connection form can be used with an explicit, experimental credential bridge:

```powershell
$env:MSRDPEX_MSTSCAX_DLL = (Resolve-Path .\target\release\ironrdpax.dll)
$env:MSRDPEX_AX_BACKEND = 'ironrdp'
$env:IRONRDP_ACTIVEX_NATIVE_MSTSC_CREDENTIAL_BRIDGE = '1'
& '<path-to-MsRdpEx-build>\Release\mstscex.exe'
```

After entering a Computer name and selecting **Connect**, the observed public
`IMsTscSecuredSettings::put_StartProgram` preflight synchronously opens the standard Windows CredUI
dialog and stops mstsc before its private RemoteApp continuation. The selected server is read from
the visible mstsc Computer field; the username and password are provided directly to the in-memory
IronRDP connection and are not written to logs, command lines, environment variables, or the Windows
credential store. The bridge preserves the account form returned by CredUI, including `DOMAIN\user`
and UPN syntax. Cancelling the dialog returns to the mstsc form. The older empty extended-settings
preflight remains only as a fallback for an observed alternate host flow. This bridge is limited to
native mstsc compatibility and does not implement RemoteApp.

When `RDP_USERNAME` and/or `RDP_PASSWORD` are present in the `mstscex.exe` process environment,
the bridge pre-populates the corresponding CredUI fields. It never auto-submits the dialog; the
operator must still approve it. These values are used only to initialize CredUI's local buffers,
are not traced or persisted, and should be supplied only through a suitably protected process
environment.

For authorized automation tests, a credential-free `.rdp` file may provide the destination while
the launcher supplies that same destination in `RDP_HOSTNAME`. When the native connection form is
absent, the bridge uses that process-local value as its destination fallback; `RDP_USERNAME` and
`RDP_PASSWORD` remain CredUI-only and must never be written to the file. This starts the IronRDP
session, but native MSTSC can still show a nonfatal error after the bridge stops its proprietary
continuation. Verify the session through the ActiveX RPC endpoint before dismissing that dialog.

In this explicit bridge mode, non-minimized native-shell container size changes become a 250 ms
debounced `IMsRdpClient9::UpdateSessionDisplaySettings` request. IronRDP uses the negotiated Display
Control dynamic channel ([MS-RDPEDISP]) for the live resolution change and falls back to its
controlled reconnect path when the server does not advertise that channel. Minimized or zero-sized
windows do not request a remote resize; restore/maximize requests the current nonzero client bounds.
The native bridge also fits the last remote frame locally while a display update is pending, so
container changes never crop it; `SmartSizing` and `ZoomLevel` remain local, aspect-preserving
presentation controls.

The control deliberately does **not** remove the native `mstsc.exe` window frame to simulate full
screen. That private host behavior includes input routing and restoration semantics; modifying the
host shell from an in-proc ActiveX control can trap the user without an exit surface. In explicit
bridge mode, native `IMsRdpClient::put_FullScreen` instead uses the documented maximized/restored
shell presentation and keeps its title bar. Outside that opt-in mode it returns `E_NOTIMPL`.

For an eligible connected full-screen embedded session, `DisplayConnectionBar` creates an
IronRDP-owned, non-activating tool window owned by the renderer's native host ancestor. The
explicit native MSTSC bridge presents the bar when the host enters full screen unless a caller has
explicitly disabled `DisplayConnectionBar`; generic embedded hosts retain the public opt-in default.
`DisableConnectionBar` suppresses it; an unpinned bar auto-hides after three seconds and can be
exposed again by moving to the renderer's top edge. Exposing it raises
`OnConnectionBarPullDown` (DISPID `0x1E`) synchronously.
Hovering the bar or one of its actions pauses that timer; an actual mouse leave resumes it. Its
visible label is `ConnectionBarText`, or the configured server when that property is empty.
The bar is re-anchored to its owner whenever it is shown or refreshed and scales its window and
controls for the owner's DPI; while visible, it also safely polls its owner geometry so host moves,
resizes, and monitor transitions reapply that owner-relative layout without subclassing the host.
A bar `WM_DPICHANGED` likewise reapplies the layout.
Its IronRDP-owned **Info** action opens a host-owned connection-information dialog only for an
actually connected session. It reports only the actual connected state, an observed remote desktop
size when available, and whether the active session has clipboard redirection; it never exposes
the server, account, credentials, certificate, negotiated security details, or remote errors.
Its **Disconnect** action asks for confirmation before calling the control's existing public
`Disconnect` operation. The confirmation is only offered for an actually connected session and
contains no endpoint, credential, certificate, or remote-error details. Cancelling it or failing
to display its IronRDP-owned dialog leaves the session connected.
The bar actions are standard tab-stop buttons: Tab and Shift+Tab cycle only visible enabled buttons,
and Escape returns focus to the renderer. These local navigation actions never raise
`OnFocusReleased`, which is reserved for a real configured remote focus-release hotkey.
The public OLE `EnableModeless` notification enables or disables only the bar's local interactive
commands while a container-owned modal dialog is active; it neither hides the bar nor changes the
RDP session. The setting is retained for a subsequently created bar.

While an IronRDP connection is genuinely starting, the control also presents an IronRDP-owned,
modeless, non-activating connection-health popup owned by the renderer. It is centered over the
renderer, scales from that child window's DPI, and polls only its own owner-relative geometry; it
does not subclass or otherwise modify an embedding host window. The generic worker currently has
no truthful reconnect-progress transition, so the popup shows only **Connecting...** and is removed
after actual connection, final disconnect/error, OLE close/deactivation, or renderer destruction.
An internal hook accepts future worker retry progress only when both a positive attempt and its
maximum are known; only then may it display **Reconnecting...** and `Attempt N of M`. It has no
Cancel action, never includes endpoints, credentials, certificates, or error detail, and never
raises a COM event.

When an actual Display Control update cannot complete in-session, IronRDP's worker reports that it
is reconnecting with the requested display size. The same popup then shows **Updating remote
display...** without a retry count or synthetic lifecycle event, and closes only after that worker
reports the replacement connection, a terminal disconnect/error, or UI teardown. `UIDeactivate`
also releases held remote input and destroys transient IronRDP-owned UI; a later successful OLE
activation recreates an eligible connection bar from the existing connected state. The renderer's
actual `WM_SHOWWINDOW` hide/show lifecycle applies the same teardown and restoration, preventing
owned UI from remaining over a hidden embedded rendering surface without changing the host window.
The renderer also reacts immediately to its actual `WM_DPICHANGED` notification by reflowing only
IronRDP-owned UI from current DPI and leaves the OLE container's suggested renderer rectangle
untouched. Its actual move, resize, cancellation, and disable notifications likewise reflow owned
windows or release held remote input without requesting a remote display resize. OLE frame or
document deactivation, client-site detachment, and renderer teardown also release held remote keys
and buttons even when the host does not deliver a separate focus-loss message. If an embedding
parent destroys the renderer directly, the final `WM_NCDESTROY` performs the same local owned-UI,
input, handle, and class-resource cleanup without disconnecting the live RDP session or inventing
lifecycle events.

In the explicit native MSTSC bridge only, the bar provides accessible **Minimize**, **Restore**,
**Full screen**, **Close**, **Pin**/**Unpin**, and **Disconnect** actions. These use documented
window operations on the verified `TscShellContainerClass`; arbitrary OLE hosts never receive
window-state or close manipulation. Full screen remains safe maximized/restored presentation with
normal window chrome, rather than borderless host mutation.
`ConnectionBarShowPinButton` controls the Pin/Unpin button visibility. The minimize and restore
button compatibility values control their corresponding actions only in the explicit native bridge;
the controls are absent for generic OLE hosts.

## Implemented Automation surface

The `IDispatch` implementation supports these classic MSTSCLib DISPIDs:

| Member | DISPID | Support |
| --- | ---: | --- |
| `Server`, `Domain`, `UserName` | 1, 2, 3 | Read/write connection settings |
| `DisconnectedText`, `ConnectingText`, `Connected` | 4, 5, 6 | Read-only lifecycle values |
| `DesktopWidth`, `DesktopHeight`, `StartConnected` | 12, 13, 16 | Read/write settings |
| `HorizontalScrollBarVisible`, `VerticalScrollBarVisible` | 17, 18 | Read-only; always disabled |
| `FullScreenTitle` | 19 | Write-only retained presentation title |
| `CipherStrength`, `SecuredSettingsEnabled` | 20, 22 | Read-only supported client capabilities |
| `Version` | 21 | Read-only IronRDP version string |
| `Connect`, `Disconnect` | 30, 31 | Starts/stops the IronRDP client worker |
| `ColorDepth` | 100 | Read/write; accepts only 16 or 32; defaults to 32-bit |
| `ExtendedDisconnectReason` | 103 | Read-only; no additional reason until the worker provides one |
| `FullScreen` | 104 | Read/write presentation state for embedded hosts; native `mstsc.exe` fullscreen entry is explicitly unsupported |
| `ConnectedStatusText` | 201 | Read/write connection-status text |

`IronRdpPassword` is an IronRDP-specific write-only Automation property used to supply the password
alongside the public `IMsTscSecuredSettings` raw interface. Reading it fails rather than returning a
secret.

For the native `mstsc.exe` host, `FullScreen` and `Ctrl`+`Alt`+`Break` (or `Pause`) return
`E_NOTIMPL` without changing the outer `TscShellContainerClass` window. Directly manipulating that
Microsoft-owned shell from the in-process control causes native host termination, so the shell's own
maximize/fullscreen behavior remains its responsibility. An embedding host that sets
`ContainerHandledFullScreen` receives `OnRequestGoFullScreen` or `OnRequestLeaveFullScreen` instead;
the control never changes an arbitrary container window.

## RDM compatibility

A source-level audit of RDM's Windows RDP host covers these ActiveX contracts:

| RDM use | ActiveX contract |
| --- | --- |
| Legacy RDP 6.1 through 11 host selection | The six published `MsRdpClient*NotSafeForScripting` class identifiers are accepted by `DllGetClassObject` and preserve their requested `IPersist` class identity. They are explicit backend aliases, not global COM registrations. |
| WinForms `AxHost` lifecycle | Windowed OLE activation, focus, sizing, the inherited `IMsRdpClient` through `IMsRdpClient10` raw interfaces, and the RDM virtual channels `RDMJump`, `RDMLog`, and `RDMCmd` are supported. |
| Connection configuration | Server, account, desktop, color, smart-sizing, keyboard, display update, gateway, audio, clipboard, CredSSP, client-device name, and backing `ConfigBuilder` settings are mapped where IronRDP provides the same behavior. |
| Events | Connecting, connected, login-complete, disconnect, fatal-error, fullscreen-leave, virtual-channel, resize, and writable confirm-close events are delivered on the creating apartment. Warning and auto-reconnect events remain unfired until an IronRDP worker produces their real state. |
| Optional RDM interfaces | Non-scriptable device, drive, camera, clipboard, monitor, and preferred-redirection interfaces expose only available backend capabilities. Empty collections and disabled capabilities never advertise RDPDR, RemoteApp, camera, or multimonitor support. |

The audit also identified RDM settings that have no IronRDP ActiveX backend: input throttling,
automatic reconnect, authentication policy, device/printer/port/smart-card
redirection, RemoteApp, audio capture, video policy, PCB, load balancing, and Microsoft workspace
extensions. Their audited AdvancedSettings vtable slots use their exact published ABI signatures,
initialize out parameters, and return `E_NOTIMPL`; the control does not report success for settings
that cannot affect the connection.

The control exposes a standard `IConnectionPointContainer` and an event connection point for the
published `IMsTscAxEvents` IID `{336D5562-EFA8-482E-8CB3-C5C0FC7A7DB6}`. Lifecycle events are delivered
on the creating apartment through a message-only dispatcher window; the RDP worker never invokes a
COM sink from its background thread. Implemented event DISPIDs are `OnConnecting`, `OnConnected`,
`OnLoginComplete`, `OnDisconnected`, `OnEnterFullScreenMode`, `OnLeaveFullScreenMode`, `OnRequestGoFullScreen`,
`OnRequestLeaveFullScreen`, `OnFatalError`, `OnAuthenticationWarningDisplayed`,
`OnAuthenticationWarningDismissed`,
`OnRemoteDesktopSizeChange`, `OnConnectionBarPullDown`, and `OnConfirmClose`. `IMsRdpClient::RequestClose` raises
`OnConfirmClose` synchronously on the creating apartment with its documented `VT_BOOL | VT_BYREF`
argument. A sink can veto closing; the control then returns `controlCloseWaitForEvents` instead of
`controlCloseCanProceed`. `OnConnected` follows completed IronRDP connection activation, and
`OnLoginComplete` follows the server's value-free Save Session Info notification rather than the
first decoded framebuffer. The connection-health popup follows only those actual lifecycle
transitions and preserves their ordering: it is shown after `OnConnecting` (`0x01`) and removed
after `OnConnected` (`0x02`) or `OnDisconnected(long)` (`0x04`). It does not synthesize
`OnAutoReconnecting(long,long,AutoReconnectContinueState*)` (`0x11`),
`OnAutoReconnected()` (`0x21`), or
`OnAutoReconnecting2(long,VARIANT_BOOL,long,long)` (`0x22`). Those events and their public
automatic (`0`), stop (`1`), and manual (`2`) continuation values remain unavailable until an
IronRDP worker produces the corresponding real state.

Worker-to-apartment events are bounded to 64 pending entries. Frame updates coalesce to the latest
frame, while lifecycle and terminal state evict only frames or static-channel data. A full queue
with no pending frame rejects a newer frame instead of falsely reporting that it was accepted.
Static-channel data is never silently dropped: if its event cannot be queued after evicting a frame,
the session fails rather than reporting a successful but incomplete channel delivery.

Each control runs its RDP client on one dedicated, module-pinned worker thread with a current-thread
Tokio runtime. That prevents a control instance from creating additional scheduler threads and keeps
all connection teardown off the host apartment.

The client input queue is independently bounded. ActiveX `Disconnect` uses a priority cancellation
signal rather than queueing a close request, so it interrupts stalled connection setup or an active
session even when ordinary renderer input is backpressured.

TLS validates the server certificate chain and name by default. The public
`IMsRdpClientAdvancedSettings4::AuthenticationLevel` selects the preconnect policy: an unset
setting or `1` requires successful validation, `2` enables the classic public certificate-warning
lifecycle, and an explicit `0` disables certificate and name validation. The explicit native-mstsc
credential-bridge mode also enables the warning lifecycle if the host leaves `AuthenticationLevel`
unconfigured; an explicit non-2 value remains authoritative. `AuthenticationLevel=0` is vulnerable
to on-path attacks and is only appropriate for an isolated development or test endpoint. On an
otherwise-invalid certificate, the control fires `OnAuthenticationWarningDisplayed` (DISPID 18),
shows a synchronous host-owned warning, then fires `OnAuthenticationWarningDismissed` (DISPID 19).
Continuing accepts only that exact certificate fingerprint for the connection; the optional
**Remember** choice stores its SHA-256 fingerprint under the IronRDP-owned per-user key
`HKCU\Software\Devolutions\IronRDP\ActiveX\TrustedCertificates`. `PublicMode` suppresses lookup of
remembered exceptions. The control intentionally does not read or write Microsoft's `Terminal
Server Client\Servers` registry values.

The warning is implemented with the operating system's `TaskDialogIndirect` UI when the host
provides the Common Controls v6 entry point. Hosts without it reject the certificate rather than
silently weakening validation.

`IMsRdpClientNonScriptable3::WarnAboutSendingCredentials` and
`WarnAboutClipboardRedirection` are retained read/write preconnect-consent settings. When enabled,
IronRDP presents separate neutral TaskDialog prompts before starting a worker: the credential
prompt only when an actual password is available to send, followed by the clipboard prompt only
when clipboard redirection is actually enabled. Cancelling either prompt leaves the control idle;
if the consent UI is unavailable, the connection fails closed. Neither prompt includes endpoint,
credential, certificate, or remote-error details. A genuine terminal connection failure fires its
existing `OnFatalError` and `OnDisconnected` events first, then shows a bounded generic failure
dialog with no remote error text.

RDPDR-style redirection warning UI remains unsupported: `ShowRedirectionWarningDialog`,
printer/device/drive redirection warnings, and DirectX redirection continue to return `E_NOTIMPL`
because this ActiveX backend does not advertise the corresponding protocol capabilities.

Every independently returned COM child object, including settings objects, empty capability
collections, clipboard capabilities, connection points, and OLE or connection enumerators, keeps
the server loaded until its final `Release`. This remains true after its parent control has
been released, which prevents MsRdpEx or another host from unloading callback code while it still
owns an interface pointer.

## Raw client interfaces

The newest five published main client interfaces are `IMsRdpClient6` through `IMsRdpClient10`; the
public MSTSCLib type library has no `IMsRdpClient11` or `IMsRdpClient12` interfaces. This crate
implements their complete inherited vtable layout, including `IMsTscAx_Redist`, `IMsTscAx`, and
`IMsRdpClient` through `IMsRdpClient5`. Querying any of those IIDs from an `IMsRdpClient10` object
returns the appropriate prefix of the same 73-slot vtable.

The raw members for server, domain, user name, connection text, desktop size, start-connected,
color depth, version, `Connect`, and `Disconnect` map to the same IronRDP settings and lifecycle as
the Automation surface. On a connected session, `IMsRdpClient9::SyncSessionDisplaySettings` and
`UpdateSessionDisplaySettings` queue IronRDP's display-control resize path; servers without that
channel use IronRDP's controlled reconnect-with-new-size fallback. Display Control layouts are
sent only after the server capabilities PDU and strictly one at a time: later requests coalesce to
the latest dimensions until the preceding deactivation-reactivation sequence completes. If server
capabilities or that sequence do not arrive within three seconds, the client reconnects using the
latest requested dimensions rather than sending overlapping layouts. The latter accepts desktop
scaling and an optional complete physical size. Rotation and separate device scale values other
than the standard `100` remain explicitly unsupported (`E_NOTIMPL`). `CipherStrength`,
`FullScreen`, `RequestClose`, and status text provide compatible defaults. The control records only
disconnect facts it owns: an explicit client disconnect uses the published
`exDiscReasonAPIInitiatedDisconnect` extended reason, while safe connector-failure categories and
server/session termination retain `exDiscReasonNoInfo` with a bounded, non-secret description from
`GetErrorDescription`. It never forwards untrusted remote error text. BSTR outputs are allocated for
the caller and unsupported output members are cleared before returning `E_NOTIMPL`.

`IOleInPlaceObject::SetObjectRects` applies both the local child window position and its OLE clip
rectangle, then repaints the existing framebuffer. It never requests a remote display resize:
embedding hosts can issue this OLE layout call repeatedly while the user moves a window. A host that
requires dynamic resolution must call the explicit session-display interface above.
When the control is UI-active, `IOleInPlaceActiveObject::TranslateAccelerator` forwards the host
message and current Shift/Ctrl/Alt state to `IOleControlSite::TranslateAccelerator`, preserving the
standard handled (`S_OK`) versus unhandled (`S_FALSE`) outcome. This keeps container accelerators
available without treating the RDP renderer as a keyboard-focus trap.
`IOleControl` is available for generic containers, but `GetControlInfo` and `OnMnemonic` return
`E_NOTIMPL`: the control supplies no local accelerator table or mnemonic. Ambient property changes
remain accepted as no-ops. `FreezeEvents(TRUE)` is nesting-counted; outgoing connection-point events
remain suppressed until every matching `FreezeEvents(FALSE)` call has been received.

When a display-control resize causes the RDP deactivation-reactivation sequence, the client
prefers the server-advertised Suppress Output off/on pair to force a full redraw. This is the
documented Refresh Rect workaround for affected Microsoft RDP servers. Servers without that
capability receive a full Refresh Rect request only when they explicitly advertise it. Until the
redraw arrives, the new framebuffer is seeded by scaling the prior frame so partial updates cannot
expose zeroed regions. Local ActiveX bounds changes also force a full GDI repaint instead of
reusing copied child-window pixels.
The same server-advertised redraw mechanism is used once after the first post-logon Save Session
Info notification, before `OnLoginComplete`, to recover a complete initial framebuffer. When
`IRONRDP_ACTIVEX_HOST_TRACE` is enabled, that recovery records
`RdpWorker::PostLogonDisplayRedraw`.
When `IRONRDP_ACTIVEX_HOST_TRACE` is enabled, a full reconnect fallback records one of
`RdpWorker::DisplayResizeFallback:DisplayControlUnavailable`,
`...:CapabilitiesTimedOut`, or `...:ReactivationTimedOut`. A successful in-session resize does
not produce a fallback marker; the required Deactivation-Reactivation Sequence can nevertheless
look like a reconnect in the remote UI.

The ActiveX GDI host deliberately does not advertise the RemoteFX bitmap codec and disables lossy
compression. It defaults to IronRDP's 32-bit color profile. Hosts can explicitly request
`ColorDepth=16` to use the lossless Interleaved-RLE bitmap route while investigating a server's
32-bit RDP 6 display-update behavior.
For an active session, `Reconnect(width, height, status)` uses IronRDP's live display-update path
and reports `controlReconnectStarted` after queuing the resize. It initializes `status` to
`controlReconnectBlocked` before rejecting an inactive, invalid, or unsupported update.
`Connect` rejects a second active request with `E_FAIL`; a pre-credential connection form remains
deferred-success until a destination and password are available. `Disconnect` returns `E_FAIL`
before an IronRDP worker exists rather than reporting a fictitious cancellation.

The class factory accepts the published `MsRdpClient6NotSafeForScripting` through
`MsRdpClient11NotSafeForScripting` class identifiers used by RDM, preserving the requested class
identity through `IPersist::GetClassID`. These are factory aliases for explicit MsRdpEx backend
selection, not registrations that replace Microsoft’s system-wide `mstscax.dll` classes.

`CreateVirtualChannels` registers comma-delimited, ASCII static-channel names before
`Connect`; later registration and option changes return `E_UNEXPECTED`. The control rejects empty,
duplicate, reserved (`drdynvc`, `cliprdr`, and `rdpsnd`), or over-seven-byte names, and accepts at
most 28 host channels so IronRDP's built-in channels remain within MS-RDPBCGR's 31-channel
negotiation limit. `SetVirtualChannelOptions` retains only the published `CHANNEL_OPTION_*` bits
and applies them to GCC negotiation. `SendOnVirtualChannel` validates the registered channel and
queues Latin-1 BSTR code units as byte data for a connected session; before connection it is an
intentional no-op matching mstscax behavior. Received bytes are delivered through
`IMsTscAxEvents::OnChannelReceivedData` with temporary caller-owned BSTRs and Automation's
right-to-left argument order.

The `AdvancedSettings` through `AdvancedSettings9`, `SecuredSettings` through `SecuredSettings3`,
and `TransportSettings` through `TransportSettings4` getters return reference-counted, non-null
settings objects with the published vtable lengths required by mstsc.exe and MsRdpEx. Returned
settings objects keep the server loaded until their final `Release`, even if their parent control
has already been released. The currently mapped members are `SmartSizing`, `EnableCredSspSupport`,
`KeyboardHookMode`, keyboard type,
subtype, and functional-key count, secured `StartProgram`/`WorkDir`, both public
`AudioRedirectionMode` slots, `GrabFocusOnConnect`, `Compress`, `RDPPort`,
`AuthenticationLevel`, and `PublicMode`,
`RedirectClipboard`, `PerformanceFlags`, and RD Gateway transport selection.
`StartProgram` and `WorkDir` retain their caller-owned BSTR values and configure IronRDP's next
Client Info PDU alternate shell and working directory. The keyboard fields configure the next GCC
Client Core Data block. Audio mode `0` enables the Windows-native RDPSND playback backend; modes
`1` (play on server) and `2` (disabled) suppress the local RDPSND channel. When requested,
`GrabFocusOnConnect` focuses the ActiveX renderer only after its first remote frame arrives.
Invalid audio modes and keyboard types return `E_INVALIDARG`.
`Compress`, `RDPPort`, and `RedirectClipboard` configure the next IronRDP connection. Clipboard
redirection creates its Windows CLIPRDR listener on the ActiveX creating apartment, where its hidden
window is serviced by the host message loop; the RDP worker receives only the thread-safe backend
factory. Disabling it omits the channel. `EnableCredSspSupport` is
applied to the next connection when explicitly set; otherwise the control preserves IronRDP's
default CredSSP-enabled security negotiation. Smart sizing fits the remote framebuffer to the
ActiveX bounds while preserving its aspect ratio; when disabled, the renderer retains the
framebuffer's native size. Both modes apply the extended `ZoomLevel` factor and repaint immediately
after either setting changes. `RedirectDirectX` reports disabled and rejects attempts to enable it. `BandwidthDetection` and
`ClientProtocolSpec` return `E_NOTIMPL` because IronRDP has no equivalent automatic-detection or
client-protocol policy; their getter outputs are initialized before failure. Gateway settings are wired
to IronRDP's MS-TSGU gateway transport. `GatewayUsageMethod` accepts direct,
explicit-gateway, and detect modes; detect selects an explicitly configured gateway eagerly because
IronRDP cannot yet retry through a gateway after a direct connection fails. Gateway credentials can
use the RDP credentials or configured gateway user credentials. Profile, prompt, smart-card,
logged-on-user, and system-default gateway-policy modes return `E_NOTIMPL` rather than silently
changing authentication or routing behavior. Every remaining setting is an explicit
`TODO(activex)` stub and returns `E_NOTIMPL`; it must not be treated as enabled.
`EnableMouse` now gates renderer mouse movement, buttons, and wheel forwarding while retaining
keyboard forwarding. `DisableRdpdr` reports disabled because this ActiveX host has no safe RDPDR
device backend; attempting to enable RDPDR returns `E_NOTIMPL` instead of claiming unsupported
drive, printer, port, smart-card, or PnP device redirection.
`IMsRdpPreferredRedirectionInfo::UseRedirectionServerName` likewise reports disabled and rejects
enabling it because IronRDP does not currently consume load-balancing redirection names. Remote
actions also return `E_NOTIMPL` until an IronRDP session-operation mapping exists.
`EnableWindowsKey` likewise controls left/right Windows-key fast-path forwarding. A key-up that
matches a key forwarded before the setting changed is preserved so the remote modifier state cannot
become stuck.
`KeyboardHookMode` accepts only the public values `0` (local), `1` (remote), and `2` (remote only
while full screen). It applies that policy to Windows-key forwarding in the renderer; invalid values
return `E_INVALIDARG`.
`PerformanceFlags` configures the next Client Info PDU with the documented wallpaper, animation,
theme, cursor, font-smoothing, and desktop-composition bitmask. Unknown bits return `E_INVALIDARG`.
`NetworkConnectionType` maps the documented modem through auto-detect values (`1`–`7`) into the
next GCC Client Core Data block; out-of-range values return `E_INVALIDARG`.
`KeyBoardLayoutStr` accepts an eight-digit hexadecimal keyboard-layout HKL and maps it into the
next GCC Client Core Data block; malformed values return `E_INVALIDARG`.

`IMsTscNonScriptable` is exposed so a host can set `ClearTextPassword` for the IronRDP connection.
The portable and binary password formats intentionally remain unsupported.
`IMsRdpClientNonScriptable3::PromptForCredentials` and its later
`PromptForCredsOnClient`/`AllowPromptingForCredentials` aliases share one non-persistent CredUI
policy. When it is enabled, `Connect` opens CredUI for a configured server with no password.
Cancelling keeps the control idle; accepted credentials remain only in the in-memory connection
configuration. `IMsRdpExtendedSettings`
retains `ZoomLevel` values from 10 through 500, accepting the host's `VT_I4` or `VT_UI4` input and
returning `VT_I4`. `ClientDeviceName` accepts up to 15 UTF-16 code units and configures the next
IronRDP connection’s client name. `DisableUdpTransport=true` is accepted because the ActiveX host
does not expose a UDP transport; attempting to enable UDP returns `E_NOTIMPL`. Other extended
properties clear getter outputs before returning `E_NOTIMPL` rather than reporting ineffective
success. The ActiveX renderer centers and scales the framebuffer by the retained zoom level,
preserves aspect ratio during smart sizing, and translates pointer coordinates through the same
viewport.

### DVC COM plugins

The IronRDP-specific `IMsRdpExtendedSettings::Property` named `IronRdpDvcPluginPaths` configures
one or more native Windows Dynamic Virtual Channel plugin DLLs, such as a WebAuthn client plugin.
Set `IRONRDP_ACTIVEX_ENABLE_DVC_PLUGINS=1` in the process environment before creating the control;
without that explicit opt-in, the property returns `E_NOTIMPL`. The BSTR value is a semicolon-delimited
list of at most 16 local absolute paths. Each path is canonicalized, must name a distinct existing
`.dll`, rejects UNC paths, and can only be changed while disconnected. An empty value clears the
configured list.

The DVC loader owns each plugin's COM objects on dedicated worker threads and bridges channel data
through IronRDP's `drdynvc` implementation. It does not grant remote code execution: the embedding
host explicitly selects the local DLLs it is prepared to load. A selected plugin that cannot load or
initialize makes connection setup fail rather than quietly connecting without its requested channel.

The following IronRDP-specific extended settings configure the next connection and reject writes after
connection setup begins: `IronRdpEnableTls`, `IronRdpAutoLogon`, `IronRdpDesktopScaleFactor`,
`IronRdpCompressionLevel`, `IronRdpClientBuild`, `IronRdpClientDirectory`, `IronRdpImeFileName`,
`IronRdpDigitalProductId`, and `IronRdpFakeEventsIntervalMinutes`. Scale factor is `0` (default) or
100 through 500; compression level is 0 through 3; the fake-event interval is 0 (disabled) through
1,440 minutes. Enabling legacy TLS is explicitly opt-in because CredSSP/NLA remains the preferred
security path. These properties are not persisted and do not expose credentials or certificate
exceptions.

## Windowed ActiveX hosting

The control implements the OLE interfaces that a fresh WinForms `AxHost` uses for in-place
activation: `IOleObject`, `IOleInPlaceObject`, `IOleInPlaceActiveObject`, `IOleControl`, and
`IPersistStreamInit`. It retains the client site, participates in the standard in-place activation
callbacks, creates and resizes a child window, returns its registered class ID and a
CoTaskMem-allocated `IronRDP ActiveX Control` user-type string, and accepts `IPersistStreamInit::InitNew`.
When a compatibility CLSID activates the control, its factory preserves that requested public class
identity through `IPersist::GetClassID` and `IOleObject::GetUserClassID`.
`IOleObject::EnumVerbs` advertises a CoTaskMem-owned primary `&Open` verb that maps to the control's
in-place activation path without dirtying persisted settings.
`IOleObject::DoVerb` accepts the public primary, show, open, UI-activate, and in-place-activate
verbs through that path; hide deactivates the child window and discard-undo-state is a no-op.
When an explicit active site is supplied, it must have the same COM identity as the retained client
site or activation fails with `E_FAIL`. The control intentionally returns `OLEOBJ_S_INVALIDVERB` for
property pages and unknown verbs because it does not provide a property-page UI. On frame activation,
the existing renderer child receives keyboard focus; deactivation never steals focus from the host.
`IOleObject::Advise`, `Unadvise`, and `EnumAdvise` support standard `IAdviseSink` registrations.
The control emits `OnViewChange(DVASPECT_CONTENT, -1)` after a rendered-frame invalidation or extent
change, `OnSave` after a successful persistence write, and `OnClose` during `IOleObject::Close`.
Its extent and misc-status contracts support `DVASPECT_CONTENT` only; unsupported drawing aspects
return `DV_E_DVASPECT` rather than claiming unavailable static rendering.
It does not emit `OnDataChange` or `OnRename` because it does not implement `IDataObject` exchange or
moniker support.
`IViewObject`, `IViewObject2`, and `IViewObjectEx` report the same content extent, opaque/solid
view status, content bounds, natural extent, and view-advise notifications. They intentionally
return `E_NOTIMPL` for detached-HDC drawing, palette enumeration, and frozen snapshots: the
windowed renderer is the only authoritative frame presentation path. Point and rectangle hit tests
operate on the supplied content bounds.
`IPersistStreamInit` also saves and loads a bounded, versioned snapshot of the backed non-secret
connection and presentation settings: server, domain, user name, connection text, desktop
dimensions, color depth, start-connected/fullscreen state, smart sizing, zoom level, client device
name, performance flags, and keyboard layout. Version-one streams remain loadable with safe
compatibility defaults. It never serializes passwords, gateway credentials, channels, RemoteApp
settings, or private Microsoft state. Loading or initializing while connected or after window
activation returns `E_UNEXPECTED` so a container cannot mutate a live session.

The child window copies each complete decoded `RgbA32` snapshot into an STA-owned, retained top-down
32-bpp DIB section and scales that surface to the ActiveX bounds. Each paint composes the black
letterbox areas and scaled frame into a retained client-size memory backbuffer, then copies that
finished image to the visible window in one GDI operation so the intermediate clear is not exposed.
Keyboard scan-code messages and mouse movement, buttons, extended buttons, and wheel input are
forwarded as RDP fast-path input. The legacy
`IMsRdpClientNonScriptable::SendKeys` method is also supported for an active session: it accepts
up to 20 `WM_KEYDOWN`-style key records, honors the scan-code and extended-key bits, and forwards
the complete batch as one RDP input transaction. It returns `E_INVALIDARG` for a negative count or
more than 20 keys, `E_POINTER` for missing nonempty arrays, and `E_UNEXPECTED` without an active
remote input session. Input state is maintained by `ironrdp-input`, including extended keys,
repeats, and key/button release on focus or capture loss, so the remote session does not retain
stuck input.
Bitmap composition accepts a standard bitmap update only when its inclusive destination bounds and
declared width/height agree and fit the framebuffer. Malformed source data, a truncated bitmap
payload inside an otherwise bounded Fast-Path PDU, invalid compressed streams, or unsupported
encodings are discarded before composition and do not produce a repaint or terminate the desktop
session. The bounded truncated-Fast-Path case queues one capability-gated full-desktop recovery
request per activation, replacing retained stale regions when the server supports Refresh Rect or
Suppress Output.
Remote pointer shape, visibility, and position updates are composited into that framebuffer,
because the GDI presenter intentionally has no separate hardware-cursor overlay. This preserves
the remote cursor through smart sizing and zoom without transferring cursor-handle ownership across
the ActiveX host boundary.

New connections enable IronRDP's Windows-native CLIPRDR backend, which synchronizes the system
clipboard with the remote session. Its window-bound listener stays on the ActiveX STA for the whole
connection while the RDP worker owns the protocol backend; shutdown removes the listener on that STA.
`IMsRdpClipboard` reports synchronization available only after that enabled connection completes
activation; its explicit sync methods succeed at that point because the backend performs
synchronization automatically. They return `E_UNEXPECTED` before connection or when clipboard
redirection was disabled for the session. The separate OLE `IDataObject` clipboard contract remains a
TODO and is not claimed as supported.

`IMsRdpClientNonScriptable5` reports one remote monitor only after an active remote framebuffer is
available and returns its `(0, 0, width, height)` bounding box. Multi-monitor mode is not
implemented and enabling it returns `E_NOTIMPL`; the control does not claim that the remote layout
matches the local display topology.

The Windows-only `ironrdp-axhost` tool at `tests\ironrdp-axhost` loads a COM server through its
`DllGetClassObject` export, so it does not need COM registration. Its default `probe` operation
provides a deterministic AxHost activation check:

```powershell
cargo build -p ironrdp-activex --release
dotnet run --project .\crates\ironrdp-activex\tests\ironrdp-axhost --configuration Release -- `
    .\target\release\ironrdpax.dll probe --json
```

The `unload` operation exercises the matching lifetime contract: it advises then unadvises the
event sink, lets AxHost deactivate and release its root COM object, and requires
`DllCanUnloadNow == S_OK` before the tool frees the DLL. A nonzero `unloadStatus` fails the probe;
the tool never forces `FreeLibrary` while the server reports `S_FALSE`.

```powershell
dotnet run --project .\crates\ironrdp-activex\tests\ironrdp-axhost --configuration Release -- `
    .\target\release\ironrdpax.dll unload --json
```

With a locally built MsRdpEx wrapper, it also verifies the selected-backend route. Set
`MSRDPEX_MSTSCAX_DLL` to the architecture-matched `ironrdpax.dll` and
`MSRDPEX_AX_BACKEND=ironrdp`, then supply the wrapper DLL and its
`CLSID_MsRdpClientNotSafeForScripting` (`{7CACBD7B-0D99-468F-AC33-22E495C0AFE5}`) to the same
tool. The `--show` option displays the hosted child window for 30 seconds, including after a
successful `connect` lifecycle test.

### Opt-in connection lifecycle smoke test

Pass `connect` to configure and invoke a real IronRDP connection through the activated control. The
harness reads these required, nonempty process-local environment variables by default:

| Variable | Used for |
| --- | --- |
| `IRONRDP_SMOKE_SERVER` | `Server` |
| `IRONRDP_SMOKE_USERNAME` | `UserName` |
| `IRONRDP_SMOKE_PASSWORD` | Write-only `IronRdpPassword` |

Passwords are never accepted as command-line arguments and no input value is printed. The harness
sets a `1024x768` desktop, invokes `Connect` through `IDispatch`, waits on the published
`IMsTscAxEvents` connection point and the `Connected` property, then calls `Disconnect` and
`Unadvise` during cleanup. It exits successfully only after it observes a connection; a lifecycle
failure, timeout, or early window close exits unsuccessfully. `--timeout <seconds>` sets a
1–600-second timeout and defaults to 30 seconds. `--server` and `--username` override their
respective environment values, while `--password-env <variable>` selects a password environment
variable without exposing its value.

Set the environment variables through an approved local secret-management mechanism, then run:

```powershell
dotnet run --project .\crates\ironrdp-activex\tests\ironrdp-axhost --configuration Release -- `
    .\target\release\ironrdpax.dll connect --timeout 60 --screenshot .\artifacts\frame.png --json
```

For the MsRdpEx route, set its two explicit backend-selection variables described above and replace
the DLL path and optional CLSID:

```powershell
dotnet run --project .\crates\ironrdp-activex\tests\ironrdp-axhost --configuration Release -- `
    <path-to-MsRdpEx.dll> {7CACBD7B-0D99-468F-AC33-22E495C0AFE5} connect --timeout 60 --json
```

For agent-driven end-to-end automation, `--json` writes exactly one bounded result object to stdout
with the operation, pass/fail status, exit code, duration, observed lifecycle event names,
connection state, optional screenshot path, and a value-free local failure category. It never
serializes credentials, server names, remote error text, or packet data. Run `ironrdp-axhost
--help-agent` for its self-contained command and output contract. `--observe <seconds>` keeps a
connected session open for bounded renderer observation; `--show` displays that session and defaults
the observation period to 30 seconds.

## Current architectural boundary

The worker translates supported Automation settings into `ironrdp-client::ConfigBuilder`, starts
`RdpClient` on a dedicated Tokio runtime, and retains the DLL until the worker has terminated.
Connection points retain sinks through `Advise`/`Unadvise`, enumerate correctly, and query the supplied
sink for the event interface IID before retaining its `IDispatch`.

This is an Automation, lifecycle, hosting, framebuffer, basic input, persistence, and static
virtual-channel foundation. It does not yet implement RemoteApp, OLE `IDataObject` clipboard exchange,
monikers, RDPDR device redirection, or arbitrary persisted designer state. Those contracts must be
added as exact ABI implementations before advertising their individual methods as supported.
