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

On Windows, pass `--smartcard` (or set `ironrdp_smartcard:i:1` in a `.rdp` file) to redirect local smart cards through WinSCard.
RPC mode (`--rpc`) enables smartcard the same way via agent connect properties (`ironrdp_smartcard` / sandbox `SmartCardRedirection`).

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
- `ironrdp_smartcard:i:<0|1>` (Windows WinSCard smartcard redirection)
- `audiomode:i:<0|1|2>`
- `desktopwidth:i:<value>`
- `desktopheight:i:<value>`
- `desktopscalefactor:i:<value>`
- `compression:i:<0|1>`

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
