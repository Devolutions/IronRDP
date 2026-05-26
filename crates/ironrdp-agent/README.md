# IronRDP agent

`ironrdp-agent` is a daemon-backed CLI for driving RDP sessions without a GUI window.

The same executable can run as a daemon or as a client. The daemon owns the real RDP connections and
exposes an HTTP/1 IPC API over a local transport. Client commands connect to the daemon, spawn it when
needed, and send one automation request at a time.

## Endpoints

On Windows, use a named pipe endpoint:

```shell
ironrdp-agent --endpoint pipe:ironrdp-agent daemon
```

On Unix-like systems, use a Unix-domain socket endpoint:

```shell
ironrdp-agent --endpoint unix:/tmp/ironrdp-agent.sock daemon
```

## Basic usage

```shell
ironrdp-agent connect rdp.example.test --username alice --domain example.test --password-env IRONRDP_AGENT_PASSWORD --desktop-size 1920x1080
ironrdp-agent connect --rdp-file session.rdp --password-env IRONRDP_AGENT_PASSWORD
ironrdp-agent sessions
ironrdp-agent status --session <SESSION_ID>
ironrdp-agent wait-frame --session <SESSION_ID> --after-frame 4 --timeout-ms 60000
ironrdp-agent mouse --session <SESSION_ID> move --x 300 --y 300
ironrdp-agent mouse --session <SESSION_ID> click --button left
ironrdp-agent keyboard --session <SESSION_ID> text --text "https://example.com"
ironrdp-agent screenshot --session <SESSION_ID> --output out.png
```

Do not put real passwords in scripts or committed files. Prefer `--password-env`.

`status --session` and `sessions` report the latest framebuffer width, height, and monotonic
`frame_sequence`. `wait-frame --after-frame <N>` waits for a newer framebuffer and is useful after
input or resize commands.

## Logging

`ironrdp-agent` uses the same tracing filter syntax as other IronRDP clients. The default level is
`warn`; use `--log-level` for a simple level override or `--log-filter` for module-specific
directives. `IRONRDP_LOG` is also honored when `--log-filter` is not set.

```shell
ironrdp-agent --log-level debug --log-file ironrdp-agent.log daemon
ironrdp-agent --log-filter "ironrdp_agent=debug,ironrdp_client=trace,ironrdp_connector=trace" --log-file ironrdp-agent.log connect rdp.example.test --username alice --password-env IRONRDP_AGENT_PASSWORD
```

When a client command auto-spawns the daemon, the `--log-level`, `--log-filter`, and `--log-file`
options are forwarded to the spawned daemon.

## Localhost RDP workflow

The repository includes a manual GitHub Actions workflow in `.github/workflows/agentic-rdp.yml`.
It builds `ironrdp-agent`, enables localhost RDP on a Windows runner, connects with
`--desktop-size WxH`, starts an interactive `AwakeCoding.PSRemoting` host in the RDP session, drives
the desktop through `ironrdp-agent`, and uploads logs/screenshots.

The workflow verifies the resolution from the agent framebuffer dimensions. Do not rely on changing
the display resolution from inside the RDP session as a substitute for the RDP connection size.

The same scenario can be run from an elevated Windows shell after building the agent:

```powershell
cargo build -p ironrdp-agent --release
.\testing\agentic-rdp\Invoke-AgenticRdpTest.ps1 -DesktopSize 1920x1080
```

The local script enables RDP and sets a temporary password on the current user, so prefer running it
on disposable CI runners.

## Live E2E tests

Live tests are opt-in and read connection details from environment variables:

```shell
$env:IRONRDP_AGENT_E2E=1
$env:IRONRDP_AGENT_E2E_HOST="..."
$env:IRONRDP_AGENT_E2E_USERNAME="..."
$env:IRONRDP_AGENT_E2E_PASSWORD="..."
$env:IRONRDP_AGENT_E2E_DOMAIN="..."
cargo test -p ironrdp-agent --test live_e2e -- --ignored
```
