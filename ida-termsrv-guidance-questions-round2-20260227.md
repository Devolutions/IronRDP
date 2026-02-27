# IronRDP TermSrv / WTS Provider — IDA Questions (Round 2, 2026-02-27)

## Context snapshot from latest strict run

Observed in `artifacts/ida-guided-configured-no-prelogon-restarts.log`:

- Session pipeline is consistent:
  - `Security4624Type10=1`
  - `RemoteConnection261=1`
  - `RemoteGraphics263=1`
  - `NotifyCommandProcessCreated=1`
  - Provider + TermSrv session proof markers are present and aligned to target session.
- Capture helper starts in target session via prelogon token path (`winlogon` desktop).
- GUI proof remains prelogon-like:
  - `Explorer=0`, `Winlogon=1`, `LogonUI=1`
  - screenshot is produced but blank/uniform.
- Retry churn was suppressed by config (no repeated `WTSEnumerateProcessesW ERROR_BAD_LENGTH` storms).

We are now trying to identify what undocumented TermService internals still gate the transition from prelogon desktop -> actual user shell (`explorer.exe`) in this custom protocol path.

---

## Questions for reverse engineering

### 1) Post-auth state machine after `NotifyCommandProcessCreated`

`IWRdsProtocolConnection::NotifyCommandProcessCreated` is observed, but shell never transitions.

- What exact internal function chain in `termsrv.dll` follows this callback for custom protocol providers?
- Is this callback only informational, or does `termsrv` expect additional conditions/callback side effects before allowing shell startup?
- Are there hidden per-connection/session flags that must be set before shell launch can proceed?

### 2) Hidden shell-gate dependencies for custom protocol providers

For provider mode (non-native protocol type), identify all required gates (documented or undocumented) that must be true before `explorer.exe` can appear:

- token/session gate
- user profile gate
- Winlogon/LSM arbitration gate
- graphics/IDD gate
- licensing gate
- policy/logon UI suppression gate

Please map each gate to concrete code locations and condition checks in `termsrv.dll`.

### 3) Required callback order and timing contract

Please recover the expected order/constraints (including timing windows) among:

- `IWRdsProtocolConnection::AcceptConnection`
- `GetClientData`
- `GetUserCredentials`
- `NotifySessionId`
- `ConnectNotify`
- `GetProtocolStatus` polling
- `NotifyCommandProcessCreated`
- `LogonNotify`
- `SessionArbitrationEnumeration`
- `DisconnectNotify` / `PreDisconnect` / `Close`

Questions:
- Which of these are hard-gating, best-effort, or advisory?
- Which missing/late calls cause silent “stuck on LogonUI/winlogon” behavior?

### 4) `GetProtocolStatus` internals (opaque `Specific` fields)

We now return:

- `ProtocolType = 2` (non-RDP custom path)
- `Length = sizeof(WTS_PROTOCOL_COUNTERS)`
- `Input.Specific = Output.Specific = 0`

Need clarity on undocumented behavior:

- Does `termsrv` interpret `Specific` as a progress heartbeat/counter and require monotonic changes?
- Are there code paths where `Specific=0` keeps the connection in a pre-shell state?
- What values/ranges are seen in Microsoft’s own non-RDP providers at equivalent lifecycle points?

### 5) `QueryProperty` hidden requirements

Current handling includes:

- `PROPERTY_TYPE_GET_FAST_RECONNECT -> 2`
- `PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID -> SID`
- `PROPERTY_TYPE_SUPPRESS_LOGON_UI -> 0`
- `PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT -> 0`
- `PROPERTY_TYPE_LICENSE_GUID -> deterministic GUID`
- `PROPERTY_TYPE_CONNECTION_GUID -> E_NOTIMPL`

Questions:
- Any undocumented property GUIDs queried only in side paths (e.g., shell arbitration) that must return success?
- Could `PROPERTY_TYPE_CONNECTION_GUID -> E_NOTIMPL` silently downgrade into no-shell behavior?
- Are there hidden semantics for property value *types* (ULONG/GUID/string) beyond HRESULT success?

### 6) `WrdsLogonErrorRedirector` semantics

Observed sequence includes:

- “Please wait for Group Policy Client”
- “Please wait for Local Session Manager”
- “Welcome”
- Then `RedirectMessage utype=16` with “The data is invalid.”

Questions:
- What does `utype=16` represent internally?
- Is this message expected/non-fatal, or a signal that blocks shell transition for custom providers?
- Which code paths emit this message, and what preconditions trigger it?

### 7) Connection callback agile-reference failure (`0x80040155`)

Observed:

- `Failed to create AgileReference for connection callback ... IID {F1D70332-D070-4EF1-A088-78313536C2D6}`
- Connection still proceeds.

Questions:
- Identify this IID/interface and its role.
- Is missing proxy registration truly harmless, or can it disable a late-stage notification needed for shell launch?
- Does this map to a COM apartment/threading mismatch that only affects non-default protocol providers?

### 8) Input handle fallback significance

Observed:

- keyboard handle open fails (`0x80070005`)
- pointer handle fallback is used

Questions:
- Is keyboard-device open failure expected in this mode?
- Could absence of a real keyboard handle block SAS/logon completion or shell unlock paths?
- What exact downstream components inspect these handles?

### 9) IDD / graphics relationship to shell readiness

Observed:

- `EnableWddmIdd enabled=true`
- `OnDriverLoad` occurs
- remote graphics signal present
- session-linked bitmap updates present

Yet shell remains prelogon-like.

Questions:
- Does `termsrv` have any hidden dependency where graphics readiness must be acknowledged through another callback/event before shell transition?
- Are there undocumented checks on *interactive desktop type* (`winsta0\default` vs `winsta0\winlogon`) tied to provider protocol state?

### 10) Token-source implications in custom protocol mode

Current behavior:

- first helper can start with `winlogon.exe` token when user token unavailable.
- later restarts were intentionally restricted to avoid churn.

Questions:
- In Microsoft’s intended flow, should custom providers ever operate long-term on winlogon token/desktop after Type10?
- Which internal event indicates it is safe/required to switch from winlogon desktop to default desktop?
- Does `termsrv` expect provider to actively re-bind/re-negotiate token/session ownership at that point?

### 11) Undocumented calls around profile/logon completion

Please locate any undocumented or lightly documented internal calls between:

- successful credentials validation
- `NotifyCommandProcessCreated`
- user-profile load completion
- shell process creation

Especially interested in:
- hidden RPCs to Winlogon/LSM/User Profile Service
- registry/policy checks specific to custom protocol stacks
- custom-protocol branch points that differ from built-in RDP provider logic

### 12) Practical breakpoint/watchpoint guidance

Please suggest an IDA/WinDbg breakpoint plan that is most likely to isolate the blocker quickly.

Requested outputs:

- exact symbol/function candidates (or signatures if unnamed)
- breakpoint conditions keyed by `connection_id/session_id`
- key structure fields to watch for state transitions
- expected vs problematic value patterns at each breakpoint

---

## Extra requests (anything undocumented that might help us)

If you discover undocumented behavior that is likely relevant, please include it even if not asked above, especially:

- hidden assumptions about `WTS_PROTOCOL_TYPE_NON_RDP`
- tolerated vs required HRESULTs for provider callbacks
- silent fallback paths that keep session in prelogon desktop without disconnecting
- policy/licensing checks that are not surfaced through clear error codes

A concise “minimum required callback/property contract for shell transition” summary would be extremely helpful.