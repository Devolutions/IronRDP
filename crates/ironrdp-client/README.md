# IronRDP client

Portable RDP client without GPU acceleration.

This is a a full-fledged RDP client based on IronRDP crates suite, and implemented using
non-blocking, asynchronous I/O. Portability is achieved by using softbuffer for rendering
and winit for windowing.

## Sample usage

```shell
ironrdp-client <HOSTNAME> --username <USERNAME> --password <PASSWORD>
```

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

[awakecoding-repository]: https://github.com/awakecoding/wireshark-rdp#sslkeylogfile

