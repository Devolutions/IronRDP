# IronRDP client

Portable RDP client without GPU acceleration.

This is a a full-fledged RDP client based on IronRDP crates suite, and implemented using
non-blocking, asynchronous I/O. Portability is achieved by using softbuffer for rendering
and winit for windowing.

## Sample usage

```shell
ironrdp-client <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

## `.rdp` file support

You can load a `.rdp` file with `--rdp-file <PATH>`.

Currently supported properties:

- `full address:s:<value>`
- `alternate full address:s:<value>`
- `server port:i:<value>`
- `username:s:<value>`
- `domain:s:<value>`
- `enablecredsspsupport:i:<0|1>`
- `gatewayhostname:s:<value>`
- `gatewayusagemethod:i:<value>`
- `gatewaycredentialssource:i:<value>`
- `gatewayusername:s:<value>`
- `GatewayPassword:s:<value>`
- `kdcproxyname:s:<value>`
- `KDCProxyURL:s:<value>`
- `alternate shell:s:<value>`
- `shell working directory:s:<value>`
- `redirectclipboard:i:<0|1>`
- `audiomode:i:<0|1|2>`
- `desktopwidth:i:<value>`
- `desktopheight:i:<value>`
- `desktopscalefactor:i:<value>`
- `compression:i:<0|1>`
- `ClearTextPassword:s:<value>`

Property precedence is:

1. CLI options
2. `.rdp` file values
3. Defaults and interactive prompts

Unknown or unsupported `.rdp` properties are ignored and do not fail parsing.

## Configuring log filter directives

The `IRONRDP_LOG` environment variable is used to set the log filter directives. 

```shell
IRONRDP_LOG="info,ironrdp_connector=trace" ironrdp-client <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

See [`tracing-subscriber`’s documentation][tracing-doc] for more details.

[tracing-doc]: https://docs.rs/tracing-subscriber/0.3.17/tracing_subscriber/filter/struct.EnvFilter.html#directives

## Support for `SSLKEYLOGFILE`

This client supports reading the `SSLKEYLOGFILE` environment variable.
When set, the TLS encryption secrets for the session will be dumped to the file specified
by the environment variable. 
This file can be read by Wireshark so that in can decrypt the packets.

### Example

```shell
SSLKEYLOGFILE=/tmp/tls-secrets ironrdp-client <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

### Usage in Wireshark

See this [awakecoding's repository][awakecoding-repository] explaining how to use the file in wireshark.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[awakecoding-repository]: https://github.com/awakecoding/wireshark-rdp#sslkeylogfile

