# IronRDP RDCleanPath

RDCleanPath PDU structure used by IronRDP and Devolutions Gateway.

This crate is part of the [IronRDP] project.

RDCleanPath carries an optional complete preconnection PDU and an optional X.224 request. When X.224
is present, the proxy performs X.224 followed by TLS. When only the preconnection PDU is present, the
proxy writes it and performs TLS immediately, leaving CredSSP and X.224 to the client.

[IronRDP]: https://github.com/Devolutions/IronRDP
