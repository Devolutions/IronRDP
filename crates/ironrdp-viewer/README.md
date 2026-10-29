# IronRDP Viewer

Portable RDP client without GPU acceleration.

This is a a full-fledged RDP client based on IronRDP crates suite, and implemented using
non-blocking, asynchronous I/O. Portability is achieved by using softbuffer for rendering
and winit for windowing.

## Prebuilt binaries

Prebuilt, checksummed archives are attached to each GitHub Release under the `ironrdp-viewer-v*`
tags. See the [Releases page](https://github.com/Devolutions/IronRDP/releases) for per-platform
download and verification instructions.

## Sample usage

```shell
ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

You can provide the hostname and credentials through environment variables instead:

```shell
RDP_HOSTNAME=<HOSTNAME> RDP_USERNAME=<USERNAME> RDP_PASSWORD=<PASSWORD> ironrdp-viewer
```

On Windows, the viewer redirects local drives:

- `--drives` redirects every drive, including drives connected later (`drivestoredirect:s:*`).
- `--dynamic-drives` redirects drives connected during the session, such as USB sticks (`DynamicDrives`).
- `--drive <NAME>,<PATH>` redirects a directory as a drive named `NAME`; repeat it for more drives.
- `--home-drive` redirects the user profile directory as a drive named `home`.

The `.rdp` `drivestoredirect` property selects drives with the mstsc syntax (`C:\;E:\;DynamicDrives`), and the legacy `redirectdrives:i:1` selects every drive when `drivestoredirect` is empty.
An explicit `redirectdrives:i:0` disables drive redirection even when `drivestoredirect` lists drives, and unsupported `drivestoredirect` entries (such as folder paths) are reported as warnings.

On Windows, pass `--smartcard` (or set `ironrdp_smartcard:i:1` in a `.rdp` file) to redirect local smart cards through WinSCard.
RPC mode (`--rpc`) enables smartcard the same way via agent connect properties (`ironrdp_smartcard` / sandbox `SmartCardRedirection`).

On Windows, pass `--printer` (or set `redirectprinters:i:1` in a `.rdp` file) to redirect the current default printer.
`ironrdp_rdpdr:i:0` overrides the file property, as in RPC mode, where the `redirectprinters` connect property does the same.
Pass `--printer-all` to redirect every local queue and printer connection, or `--printer-queue <queue>[,<driver>]` once per queue to redirect a named subset.
With `--printer` or `--printer-all`, `--printer-queue` entries only override drivers and must name one.
`--printer-default <queue>` picks the remote default printer; otherwise it is the local default queue.
Jobs are sent to the local queue as `RAW` data, so the remote host needs a printer driver with the announced name, which is the local queue's driver unless `--printer-queue` overrides it.
Printing preferences changed on the server are saved per server under `%APPDATA%\ironrdp-agent\printer-cache` (or `IRONRDP_AGENT_PRINTER_CACHE`) and restored on the next connection.
Easy Print/XPS is not supported.

On Windows, pass `--serial COM3` or `--parallel LPT1` (repeatable, or `all`) to redirect local serial and parallel ports; `redirectcomports:i:1` in a `.rdp` file redirects all of them.
The ports present when the session connects are announced under their local names, such as `COM3`, which Windows servers use for the port inside the session.
A redirected port is held open locally only while a remote application has it open.

On Windows, pass `--camera <NAME>` (repeatable; `*` for every camera) to redirect local cameras, or set `camerastoredirect` in a `.rdp` file.
Raw camera frames are encoded to H.264 unless `--camera-raw` is passed.

Audio plays on the system default output device, followed when it changes; `--audio-output-device` selects another one by ID or name (`--list-audio-devices` shows the choices).
`--microphone` redirects the default input device, or the one `--audio-input-device` selects.
`--audio-latency <MS>` sets how much audio is buffered ahead of the speakers (100 ms by default); raise it on networks with high jitter.

The graphics pipeline (MS-RDPEGFX) is used by default with H.264, including AVC444, and MS-RDPEVOR video optimized remoting is accepted; both decode with the Windows H.264 decoder.
Build with `--features openh264` to use the bundled OpenH264 sources instead (required on other platforms), and pass `--egfx=false` to keep graphics on the legacy bitmap path.

To open the basic console of a local Hyper-V VM with the current Windows account:

```powershell
ironrdp-viewer localhost --vmconnect <VM_ID> --vmconnect-basic --vmconnect-current-user
```

List VM IDs with `Get-VM | Select-Object Name, Id`.
On local Windows VMConnect connections, the viewer accepts Hyper-V's private frame-buffer DVC and renders its shared-memory DIB.

## Agent RPC host

The viewer can host the same local RPC protocol used by `ironrdp-agent`, while keeping its visible
window. Start the viewer before the agent so it claims the agent's default local endpoint:

```shell
ironrdp-viewer --rpc
ironrdp-agent connect --server <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

The RPC host uses the default `ironrdp-agent-<uid>.sock` endpoint on Unix or
`\\.\pipe\ironrdp-agent-<user>` on Windows. Override it with `--rpc-endpoint` on the viewer and
the same `--endpoint` value on the agent. The GUI and agent share one RDP session, including its
framebuffer and input. Close the viewer window to stop the host.

## `.rdp` file support

You can load a `.rdp` file with `--rdp-file <PATH>`.

Currently supported properties:

- `full address:s:<value>`
- `alternate full address:s:<value>`
- `server port:i:<value>`
- `username:s:<value>`
- `ClearTextPassword:s:<value>`
- `domain:s:<value>`
- `enablecredsspsupport:i:<0|1>`
- `gatewayhostname:s:<value>`
- `gatewayusagemethod:i:<value>`
- `gatewaycredentialssource:i:<value>`
- `gatewayusername:s:<value>`
- `GatewayPassword:s:<value>`
- `kdcproxyurl:s:<value>` (also `KDCProxyURL:s:<value>`)
- `kdcproxyname:s:<value>`
- `alternate shell:s:<value>`
- `shell working directory:s:<value>`
- `redirectclipboard:i:<0|1>`
- `drivestoredirect:s:<value>` (Windows drive redirection)
- `redirectdrives:i:<0|1>` (Windows drive redirection)
- `ironrdp_smartcard:i:<0|1>` (Windows WinSCard smartcard redirection)
- `ironrdp_vmconnect:s:<VM_ID>`
- `ironrdp_vmconnect_basic:i:<0|1>`
- `ironrdp_vmconnect_current_user:i:<0|1>` (Windows current-account host authentication)
- `audiomode:i:<0|1|2>`
- `audiocapturemode:i:<0|1>`
- `audioqualitymode:i:<0|1|2>`
- `ironrdp_audiooutputdevice:s:<ID or name>`
- `ironrdp_audioinputdevice:s:<ID or name>`
- `ironrdp_audiolatency:i:<milliseconds>`
- `desktopwidth:i:<value>`
- `desktopheight:i:<value>`
- `desktopscalefactor:i:<value>`
- `compression:i:<0|1>`
- `camerastoredirect:s:<value>` (Windows camera redirection; `*`, `;`-separated symbolic links or friendly names, `-` to exclude)

Property precedence is:

1. CLI options
2. Environment variables
3. `.rdp` file values
4. Defaults and interactive prompts

Unknown or unsupported `.rdp` properties are ignored and do not cause parsing failures. Parse
issues are reported to stderr.


The `IRONRDP_LOG` environment variable is used to set the log filter directives. 

```shell
IRONRDP_LOG="info,ironrdp_connector=trace" ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

See [`tracing-subscriber`'s documentation][tracing-doc] for more details.

[tracing-doc]: https://docs.rs/tracing-subscriber/0.3.17/tracing_subscriber/filter/struct.EnvFilter.html#directives

## Support for `SSLKEYLOGFILE`

This client supports reading the `SSLKEYLOGFILE` environment variable.
When set, the TLS encryption secrets for the session will be dumped to the file specified
by the environment variable. 
This file can be read by Wireshark so that in can decrypt the packets.

### Example

```shell
SSLKEYLOGFILE=/tmp/tls-secrets ironrdp-viewer <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

### Usage in Wireshark

See this [awakecoding's repository][awakecoding-repository] explaining how to use the file in wireshark.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[awakecoding-repository]: https://github.com/awakecoding/wireshark-rdp#sslkeylogfile
