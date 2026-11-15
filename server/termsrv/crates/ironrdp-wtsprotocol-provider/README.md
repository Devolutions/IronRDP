# IronRDP WTS Protocol Provider

Windows-specific Remote Desktop Services protocol provider implementation backed by IronRDP.

This crate implements `IWRdsProtocolManager`, `IWRdsProtocolListener`, and `IWRdsProtocolConnection` for side-by-side protocol registration.

## Scope (current)

- Provider object model and session registry
- Provider-owned protocol manager/listener control flow (no built-in manager/listener delegation)
- Listener lifecycle worker-thread dispatch
- Provider/companion control bridge over named-pipe IPC (`IRONRDP_WTS_CONTROL_PIPE`)
- Connection lifecycle state machine with transition validation
- CredSSP credential handoff from the companion service to Winlogon
- Caller-buffer licensing capability negotiation for the RDS grace-period path
- Input/video handle acquisition for keyboard, mouse, and video device paths
- Provider-owned native channel transport for NOW, WebAuthn, cameras, RDPDR, and RDPSND; unlisted endpoints are rejected
- In-proc COM class factory and DLL exports (`DllGetClassObject`, `DllCanUnloadNow`)
- Side-by-side registration, verification, diagnostics, and backup/restore helpers

For VM bring-up, follow the [companion deployment instructions] and [IDD setup instructions].
The [side-by-side guide](./SIDEBYSIDE_SETUP.md) covers provider registration and recovery, not a complete server deployment.

[companion deployment instructions]: ../ironrdp-termsrv/README.md#deploy-to-a-test-vm-psremoting
[IDD setup instructions]: ../ironrdp-idd/README.md#set-up-a-test-vm

## Session lifetime

Windows session arbitration can detach a connection from a temporary session before attaching it to an existing user session.
`DisconnectNotify` releases session-owned device handles but preserves the transport and connection callback for reassignment.
`Close` terminates the transport and makes the connection permanently unavailable.
The listener releases closed connections' callbacks on its next poll, without waiting for a companion `ConnectionBroken` event.
Windows still controls single-session policy and session selection.

CredSSP credentials are delivered through `GetClientData` and `GetUserCredentials`, not the listener's connection settings.
TermService converts connection settings to legacy buffers that accept only 20 username, 17 domain, and 14 password UTF-16 units; longer passwords otherwise fail before session creation.

## Input and graphics handle notes

`GetInputHandles` opens fresh keyboard and pointer device handles, including `GLOBALROOT` fallbacks, and transfers ownership to TermService.
`GetVideoHandle` returns the handle supplied by the driver-load callback, or `NULL` while that handle is unavailable.
The provider does not support environment overrides for individual device paths.

## Virtual channel routing status

`IWRdsProtocolConnection::CreateVirtualChannel` supports the dynamic NOW, WebAuthn, and camera endpoints and the static `RDPDR` and `RDPSND` endpoints through the companion's connection-local broker.
The experimental [RemoteApp opt-in] adds static `RAIL` and the internal `railenc` window encoder.
It requires a matching companion and provider DLL.
Initial-application authorization permits the trusted `rdpinit.exe` bootstrap without arguments and [published application aliases] on authenticated RemoteApp connections.
The query writes into TermService's caller-owned UTF-16 buffers, whose capacities are byte counts.
Metadata reads and native channel opens use the provider identity without changing the native RPC thread's impersonation token.
Connection-type queries identify RemoteApp sessions to TermService while desktop connections retain the native default.
Camera device names come from the client's MS-RDPECAM announcements, so the companion authorizes dynamic names against the connection's current devices rather than a fixed prefix.
Unlisted dynamic endpoints fail; unsupported static endpoints return `E_NOTIMPL`.
See the [native channel limits and camera status] before enabling redirection.
After peer acceptance, the provider opens a private overlapped message-pipe handle and transfers ownership to TermService.
TermService duplicates it into the authorized WTS caller and closes the original.
The companion reconstructs native read headers and closes the transport when the assigned session changes.

[native channel limits and camera status]: ../ironrdp-termsrv/README.md#native-dynamic-channels
[RemoteApp opt-in]: ../ironrdp-termsrv/README.md#remoteapp

Calling `WTSVirtualChannelOpenEx` from this callback recursively asks TermService to create the same channel, exhausting its threads and the VM's memory during logon.
Do not use WTS client APIs to implement the provider callback.

This restriction does not disable channel processors implemented directly in `ironrdp-termsrv`, such as its opt-in clipboard helper.

## Published RemoteApp applications

Publish applications through Windows RDS and request them as `||alias`.
The provider reads the existing `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Terminal Server\TSAppAllowList\Applications\<alias>` configuration.
It requires an absolute, unquoted `Path`; direct executable requests and unpublished aliases remain denied, except for the trusted bootstrap.
Global allow-unlisted-programs settings do not bypass these checks.
Connection Broker publication sources and document associations are not supported.

The [Windows publication properties] control arguments: `CommandLineSetting` 0 discards client arguments, 1 passes them through, and 2 replaces them with `RequiredCommandLine`.
A missing or empty `SecurityDescriptor` allows all authenticated RemoteApp users.
Otherwise, Windows `AccessCheck` checks read-data access against the RPC caller's impersonation token, never the provider's process token.
Malformed or unreadable policy fails the query rather than removing restrictions.

[published application aliases]: #published-remoteapp-applications
[Windows publication properties]: https://learn.microsoft.com/windows/win32/termserv/win32-tspublishedapplication

## Protocol manager CLSID

`{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}`

The install scripts use this CLSID by default and wire it into both:

- `HKLM\SOFTWARE\Classes\CLSID\{...}\InprocServer32`
- `HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\<Listener>\LoadableProtocol_Object`
