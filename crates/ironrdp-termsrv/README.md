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

- Environment variables: `IRONRDP_RDP_USERNAME`, `IRONRDP_RDP_PASSWORD`, `IRONRDP_RDP_DOMAIN`
  - Expected RDP credentials used for authentication.
  - `IRONRDP_RDP_USERNAME` and `IRONRDP_RDP_PASSWORD` are required for a successful CredSSP/NLA handshake.
  - Use down-level names as `IRONRDP_RDP_USERNAME=Administrator` with `IRONRDP_RDP_DOMAIN=IT-HELP`.
  - For UPNs, keep the full UPN in `IRONRDP_RDP_USERNAME` and leave `IRONRDP_RDP_DOMAIN` empty.
  - The interactive capture helper receives an allowlisted environment that excludes these credentials.

Completed capture frames are read on a dedicated task so cancelled display polls cannot corrupt frame boundaries.

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

Supply `RDP_USERNAME` and `RDP_PASSWORD` through the environment, then run from the repository root:

```powershell
.\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1 -CaptureIpc shm
```

Notes:

- The deployment script does not embed the RDP password in the scheduled task definition.
  It writes the password to a restricted file on the VM, uses it once to start `ironrdp-termsrv`, then deletes it.
- The default host is `IT-HELP-TERMSRV`; both remoting and RDP use the environment-supplied identity.
- To use separate credentials, override `-Username`, `-RdpUsername`, `-PasswordEnvVar`, and `-RdpPasswordEnvVar`.
  Avoid plaintext password arguments in shell history.
- The startup task persists, but its credential handoff is consumed once.
  Redeploy after a later reboot to restore companion authentication credentials.

### Connect with ironrdp-agent

Use `ironrdp-agent`, not `mstsc` or MsRdpEx, to connect to this VM.
It reads `RDP_USERNAME` and `RDP_PASSWORD` directly, without password files or command-line secrets.
Native Windows RDP uses port 3389; the IronRDP side-by-side provider uses port 4489.

```powershell
.\crates\ironrdp-termsrv\scripts\connect-agent.ps1
ironrdp-agent status
```

Use `-Port 4489` for the custom provider.
Connections are asynchronous; inspect `status` before requesting a screenshot.
If TLS validation fails, verify the reported SHA-256 certificate fingerprint over authenticated PSRemoting before using `ironrdp-agent cert trust HOST:PORT SHA256`.
Pin the server certificate rather than disabling daemon-wide TLS checks.
For driver prerequisites and target-only signing trust, follow the [IDD setup instructions](../ironrdp-idd/README.md#set-up-it-help-termsrv).

## Local control-plane smoke test (no TermDD required)

You can validate the control-pipe command flow without `mstsc` by running `ironrdp-termsrv` and the smoke-test client under LocalSystem or NetworkService.
The pipe rejects interactive user accounts because it transports delegated credentials.

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
