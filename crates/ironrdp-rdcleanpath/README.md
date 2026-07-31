# IronRDP RDCleanPath

RDCleanPath PDU structure used by IronRDP and Devolutions Gateway.

This crate is part of the [IronRDP] project.

RDCleanPath version 1 carries the client X.224 request so the proxy can perform X.224 followed by TLS.
Version 2 is a strict wire-format superset that adds an opaque server preconnection PDU.
For version 2, the proxy forwards that PDU unchanged, performs TLS immediately, and leaves CredSSP and X.224 to the client over the established stream.

[IronRDP]: https://github.com/Devolutions/IronRDP
