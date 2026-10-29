# IronRDP Connector

Abstract state machine to drive an RDP connection sequence.

## Microsoft Entra session authentication

Call `ClientConnector::with_entra_auth()` before negotiation to require `PROTOCOL_RDSAAD`.
This overrides the TLS-only, CredSSP, and standard-security configuration choices without permitting fallback.
Generate a 2048-bit RSA PoP key, acquire an RDP access token bound to that key, and acquire a fresh AAD nonce for each handshake.
Complete TLS with target certificate and hostname validation before calling `mark_security_upgrade_as_done()`.

Drive the connector with `Sequence::next_pdu_hint()` and `Sequence::step()` until `entra_server_nonce()` returns a nonce.
Create a signed compact JWS RDP assertion bound to that server nonce as specified by [MS-RDPBCGR] sections 2.2.18.2.1 and 5.4.5.4.1.
The assertion requires the access token, resource URI, timestamp, server nonce, public PoP key, and AAD nonce; an ordinary OAuth bearer token is not a substitute.
Call `submit_entra_assertion(assertion, output)`, send the resulting JSON over the same TLS connection, and clear the output buffer.
Resume stepping to receive the authentication result; only `S_OK` advances to Basic Settings Exchange.
Other results abort with `ConnectorErrorKind::EntraAuth`, preserving the server's HRESULT.
The subsequent Client Info PDU omits the configured password or smart-card PIN and clears password-based autologon flags.

This is a sans-I/O session-authentication exchange, not a WAM token provider.
Applications using `ironrdp-client` can enable its optional `entra-auth` feature to acquire a PoP-bound token through the documented browser authorization-code flow and drive this exchange.
The connector itself does not acquire tokens, refresh them, invoke WAM, or permit fallback.
Applications using the connector directly remain responsible for token acquisition, PoP key storage, assertion signing, claim correctness, TLS validation, and driving this phase.
The connector bounds each JSON message to 64 KiB as a local resource policy, appends a trailing NUL terminator to outbound assertions as required by Windows `termsrv`, accepts optional NUL terminators and fragmented or coalesced JSON objects on input, and does not retain or log submitted assertions.
After successful authentication, it accepts JSON whitespace before the next X.224 response.
Live Entra host interoperability has not been verified.
InteractiveEntraAuth DVC, browser prompts, AVD feed discovery, and cloud broker orchestration are not implemented here.

This crate is part of the [IronRDP] project.

## Experimental Remote Credential Guard

The `remote-guard` feature connects CredSSP's live Kerberos context to the [MS-RDPEAR] channel.

An embedder enables RCG before sending the initial negotiation request:

```rust
use ironrdp_connector::{ClientConnector, Config, ConnectorResult};
use ironrdp_dvc::DrdynvcClient;
use std::net::SocketAddr;

fn rcg_connector(config: Config, client_addr: SocketAddr) -> ConnectorResult<ClientConnector> {
    // Add other dynamic channels to this carrier before passing it to the connector.
    ClientConnector::new(config, client_addr).with_remote_guard(DrdynvcClient::new())
}
```

NTLMv2 redirection is separately opt-in: use `with_remote_guard_ntlm(DrdynvcClient::new(), None)` instead of `with_remote_guard`.
To restrict redirection to NTLMv2 (rejecting Kerberos operations on the channel so remote Kerberos attempts fall back to NTLMv2), use `with_remote_guard_ntlm_only`.
`None` binds redirected requests to the configured username and domain without guessing aliases.
Pass `Some(sspi::rcg::NtlmRedirectionNames { user, domain })` to bind them to an explicit SAM username and NetBIOS domain for the same account.
The SSPI implementation keeps the NT password hash locally and emits a session-bound opaque reference in the NTLM supplemental credential; smart-card identities omit that supplement.
It does not export the password or NT hash.

Set `config.enable_credssp = true` and pass `Some(KerberosConfig)` to the blocking or async `connect_finalize` helper.
Use the target server's hostname, a reachable KDC or KDC proxy, and the client computer name; an explicit KDC URL avoids relying on optional DNS discovery.
The embedder must verify the TLS server identity before calling `mark_as_upgraded`.
The helpers retain the completed CredSSP context and attach AuthRedirection before the RDP settings exchange.
Keep the returned static channels in the active session, as for other DVCs, and terminate the connection on channel-processing errors.

Custom drivers use `CredsspSequence::init_for_connector` and call `ClientConnector::complete_credssp(sequence)` only after writing the final authentication message and accepting any early-auth result.
Async custom drivers can use `perform_credssp_with_context` to retain that sequence.
Do not use `mark_credssp_as_done` for RCG: bypassing the context handoff is rejected.

The connector requires the server's redirected-authentication support flag and a CredSSP protocol selection.
It never falls back to TLS-only logon, NTLM for initial authentication, or password delegation; Client Info omits both passwords and smart-card PINs.
Auto-reconnect is unsupported.
The supplied DRDYNVC carrier and each complete AuthRedirection message are limited to 1 MiB.

The backend uses the Windows-native NDR profile and negotiates Kerberos and NTLM package version zero independently.
The Kerberos operations are `CreateApReqAuthenticator`, `DecryptApReply`, `UnpackKdcReplyBody`, `ComputeTgsChecksum`, `BuildEncryptedAuthData`, and `PackApReply`.
NTLM redirection uses supplemental credentials to dispatch `Lm20GetNtlm3ChallengeResponse` for NTLMv2 and `CompareCredentials`.
The NTLMv1 operations `CalculateNtResponse` and `CalculateUserSessionKeyNt` remain disabled.
Unavailable credentials or calls, invalid operation arguments, and out-of-range `SkewTime` receive explicit failure statuses, not successful empty results.
Invalid inner packets, unauthorized packages, message-protection failures, and response-limit failures immediately drop the live context.
Invalid outer framing also disables the channel; the embedder must terminate the failed connection to release its remaining state.

Fresh-logon Windows desktop access and redirected NTLMv2 authentication (including SMB administrative-share access by IP) have been verified under strict TLS.
Reconnecting to an existing target logon retains its previous credential binding and does not invoke the new connection's NTLMv2 callback; use a fresh target logon.
Outbound Kerberos service-ticket redirection, computer-account ticket selection, and user-to-user Kerberos remain unverified against a live target after the service-key and skew fixes in `sspi`.
The NTLM implementation requires `ServerName` to contain the AV pairs used by [MS-NLMP] section 3.3.2; [MS-RDPEAR] section 2.2.2.2.2 instead describes a hostname, so plaintext hostnames fail closed pending native evidence.
The supplemental credential omits the password-derived credential key, so it does not provide remote DPAPI credential-key support.
The private native `ProtectCredential` call remains unsupported.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-NLMP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-nlmp/
[MS-RDPEAR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpear/
[MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
