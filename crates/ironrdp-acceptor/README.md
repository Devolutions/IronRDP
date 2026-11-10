# IronRDP Acceptor

State machines to drive an RDP connection acceptance sequence.

For now, it requires the [Tokio runtime](https://tokio.rs/).

CredSSP authenticates against the configured credentials by default.
On Windows, select `CredsspBackend::Windows` with `Acceptor::set_credssp_backend` to authenticate local and domain accounts through Windows Negotiate instead.
This backend requires CredSSP version 5 or newer and password credential delegation; credential-less, smart card, and Remote Credential Guard delegation are not supported.
The application remains responsible for authorizing the delegated credentials before granting access to a session.

## Virtual channels

Capability re-exchange preserves virtual-channel traffic in `AcceptorResult::input_events`.
Restore `AcceptorResult::static_channels` before replaying those frames in order.
The input backlog is limited to 16 MiB of encoded frames; exceeding it fails acceptance rather than dropping data.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
