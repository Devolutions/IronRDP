# IronRDP TermSrv

Companion control-plane service for `ironrdp-wtsprotocol-provider`.

Current scope:

- Hosts a Windows named-pipe server for provider control commands.
- Accepts `StartListen`, `StopListen`, `WaitForIncoming`, `AcceptConnection`, and `CloseConnection` commands.
- Binds a TCP listener for `StartListen` and queues accepted sockets as incoming connection IDs.
- Returns control responses (`ListenerStarted`, `ListenerStopped`, `IncomingConnection`/`NoIncoming`, `ConnectionReady`, `Ack`).

The full IronRDP transport/data-plane integration is still in progress.

## Configuration

- Environment variable: `IRONRDP_WTS_CONTROL_PIPE`
  - Named pipe name (with or without `\\.\pipe\` prefix).
  - Default when unset: `IronRdpWtsControl`.

- Environment variable: `IRONRDP_WTS_LISTEN_ADDR`
  - TCP bind address for side-by-side incoming sockets.
  - Default when unset: `0.0.0.0:4489`.

- Environment variables: `IRONRDP_RDP_USERNAME`, `IRONRDP_RDP_PASSWORD`, `IRONRDP_RDP_DOMAIN` (optional)
  - Expected RDP credentials used for authentication.
  - When `IRONRDP_RDP_USERNAME` + `IRONRDP_RDP_PASSWORD` are set, the server advertises Hybrid security (CredSSP/NLA).
  - When unset, the server falls back to TLS-only.

## Build

```powershell
cargo build -p ironrdp-termsrv --release
```

## Run (debug)

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControl"
cargo run -p ironrdp-termsrv
```

## Deploy to a test VM (PSRemoting)

For unattended deploy (no password prompt), set the VM admin password in an environment variable and run:

```powershell
$env:IRONRDP_TESTVM_PASSWORD = '<vm-admin-password>'

# Optional: if the RDP password differs from the PSRemoting password.
$env:IRONRDP_TESTVM_RDP_PASSWORD = '<rdp-password>'

powershell -NoProfile -ExecutionPolicy Bypass \
  -File .\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 \
  -Hostname IT-HELP-TEST \
  -Username 'IT-HELP\Administrator' \
  -CaptureIpc shm \
  -RdpUsername 'Administrator' \
  -RdpDomain 'IT-HELP'
```

Notes:

- The deployment script does not embed the RDP password in the scheduled task definition. It writes the password to a restricted file on the VM, uses it once to start `ironrdp-termsrv`, then deletes it.
- If you omit `-RdpPassword`, the script attempts to read the password from `env:IRONRDP_TESTVM_RDP_PASSWORD` and falls back to `env:IRONRDP_TESTVM_PASSWORD` for convenience (override with `-RdpPasswordEnvVar`).
- Prefer `env:IRONRDP_TESTVM_RDP_PASSWORD` over passing `-RdpPassword` on the command line to avoid leaking credentials via shell history.

Then connect with the IronRDP client:

```powershell
$env:IRONRDP_LOG = 'debug'
target\debug\ironrdp-client.exe --log-file .\target\tmp\ironrdp-client.log \
  IT-HELP-TEST:4489 -u Administrator -d IT-HELP -p $env:IRONRDP_TESTVM_PASSWORD
```

## Local control-plane smoke test (no TermDD required)

You can validate the control-pipe command flow without `TermService`/`mstsc` by running
`ironrdp-termsrv` locally and driving commands directly through named-pipe IPC.

Example:

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControlLocal"
$env:IRONRDP_WTS_LISTEN_ADDR = "127.0.0.1:4496"
cargo run -p ironrdp-termsrv
```

In a second shell:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-control-pipe-local.ps1 \
  -PipeName IronRdpWtsControlLocal \
  -ListenerName IRDP-Tcp \
  -PortNumber 4496
```

Expected output includes:

- `listener_started`
- `incoming_connection`
- `connection_ready`
- `listener_stopped`
