# IronRDP DVC

RDP Dynamic Virtual Channel (DVC) support.

This crate is part of the [IronRDP] project.

Configure `DrdynvcClient::with_max_transport_message_size` before wrapping the client in a static virtual channel to bound inbound carrier reassembly.
The limit also applies when processing an unframed carrier directly.
It excludes the SVC header and is independent of each dynamic processor's `DvcProcessor::max_message_size` limit, since one dynamic message can span multiple carriers.
Both limits default to unlimited and are local resource policies, not negotiated protocol values.

[IronRDP]: https://github.com/Devolutions/IronRDP
