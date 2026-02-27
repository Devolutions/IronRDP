# Provider strict session-proof findings (2026-02-26)

## Scope

This note summarizes the investigation of provider-mode strict proof failures while running:

- `crates/ironrdp-termsrv/scripts/e2e-test-screenshot.ps1 -Mode Provider -StrictSessionProof -SkipBuild`

The target proof gate was `Security 4624` with `LogonType=10` for the configured RDP user.

## Repro summary

Across repeated strict runs, the observed provider callback sequence was:

1. `ConnectNotify`
2. `GetProtocolStatus` (repeated polling)
3. immediate `PreDisconnect reason=17`

Expected progress markers such as `SUPPRESS_LOGON_UI`, `NotifyCommandProcessCreated`, and stable GUI-session proof were not reached in failing runs.

## Compared evidence

Known-good callback progression (historical):

- `artifacts/remote-logs-20260225-171711/wts-provider-debug.key.log`

Failing progression (latest analyzed):

- `artifacts/remote-logs-20260226-203821/wts-provider-debug.key.log`
- `artifacts/remote-logs-20260226-203821/wts-provider-debug.log`
- `artifacts/remote-logs-20260226-203821/ironrdp-termsrv.log`
- `artifacts/remote-logs-20260226-203821/remote-connection-signals.log`

Host telemetry corroborated repeated LocalSessionManager disconnect reason 17, with no corresponding strict-proof success signal (`4624` type 10) for the target user.

## Code-level actions taken

The following provider-side behavior was tested/reset during investigation:

- `GetLogonErrorRedirector`: return redirector object (passive behavior)
- `WrdsLogonErrorRedirector::RedirectStatus`: keep `NOT_HANDLED`
- `GetProtocolStatus`: keep `Specific=0` (baseline)

Related strict harness and companion updates were also validated (SAS auto-send path, screenshot timeout controls, stricter proof gateing).

## Environment finding (probable blocker)

The VM is in Windows activation notification mode:

- `slmgr /xpr`: notification mode
- `slmgr /dli`: notification with reason `0xC004F034`
- `slmgr /ato`: activation failed (`0x87E10BC6`)

This is the strongest external blocker currently identified for interactive logon completion and strict session-proof success, independent of provider callback tweaks.

## Current conclusion

Provider strict proof remains failing with stable signature (`PreDisconnect reason=17`, no `4624` type 10) after callback behavior resets and service restarts.

At this point, VM licensing/activation health should be treated as a prerequisite to further provider strict-proof validation.
