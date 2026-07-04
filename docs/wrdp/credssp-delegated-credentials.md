# CredSSP delegated credentials handoff

`ironrdp-acceptor` already exposes credentials sent later in the TLS
SecureSettingsExchange path. CredSSP/Hybrid authentication completes earlier, so
servers that delegate final account checks after the protocol handshake also need
access to the decrypted `TSPasswordCreds` produced by the CredSSP server state
machine.

This change carries the delegated identity from the CredSSP sequence into
`AcceptorResult::credentials`, matching the existing ClientInfoPdu handoff shape.

This is useful for servers that follow the xrdp-sesman multi-user architecture
model: the RDP protocol stack authenticates the transport, then the embedding
server delegates account authorization and session launch to a separate service.
