# IronRDP SVC

IronRDP traits to implement RDP static virtual channels.

This crate is part of the [IronRDP] project.

Processors can override `SvcProcessor::max_message_size` to bound inbound reassembly before fragments are retained.
`StaticVirtualChannel::new` captures this local limit, excluding the channel PDU header, and checks both advertised plaintext length and accumulated wire bytes.
The default is unlimited; compressed messages are subject to the same configured bounds.

[IronRDP]: https://github.com/Devolutions/IronRDP
