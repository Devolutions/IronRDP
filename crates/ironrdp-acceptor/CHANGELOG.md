# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.9.0...ironrdp-acceptor-v0.10.0)] - 2026-07-10

### <!-- 1 -->Features

- Negotiate the MCS message channel ([#1347](https://github.com/Devolutions/IronRDP/issues/1347)) ([efa5732805](https://github.com/Devolutions/IronRDP/commit/efa573280572f3c0f0270a40ae51a154562706cc)) 

  Updates the handshake to properly negotiate the MCS message channel by advertising Extended Client Data Blocks support and, when requested by the client, allocating/joining the message channel and surfacing its ID in AcceptorResult. This enables server-initiated PDUs that must use the message channel (e.g., network auto-detect) to have a valid transport.

- Expose the client's keyboard layout on AcceptorResult ([#1397](https://github.com/Devolutions/IronRDP/issues/1397)) ([5ca84a5724](https://github.com/Devolutions/IronRDP/commit/5ca84a5724f48093193e39a3097c4f4987d64bbe)) 

- Honor the client-requested desktop size ([#1373](https://github.com/Devolutions/IronRDP/issues/1373)) ([d471bd066f](https://github.com/Devolutions/IronRDP/commit/d471bd066f303df22f4767801fd97ecdbf527869)) 

  Adds an opt-in server/acceptor knob to negotiate the RDP session desktop size using the client’s originally requested resolution (from GCC Client Core Data) so the server can start at the client’s native size without a Deactivation–Reactivation resize round trip.

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-async` public dependency to 0.10

- [**breaking**] Update `ironrdp-connector` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.8.0...ironrdp-acceptor-v0.9.0)] - 2026-05-27

### <!-- 4 -->Bug Fixes

- Send RDP_NEG_FAILURE on security protocol mismatch ([#1152](https://github.com/Devolutions/IronRDP/issues/1152)) ([02b9f4efbb](https://github.com/Devolutions/IronRDP/commit/02b9f4efbbe634a50efa0601f30e0a2096a6f78e)) 

  When the client and server have no common security protocol, the
  acceptor now sends a proper `RDP_NEG_FAILURE` PDU before returning an
  error, instead of dropping the TCP connection.

### <!-- 1 -->Features

- Expose received client credentials in AcceptorResult ([#1155](https://github.com/Devolutions/IronRDP/issues/1155)) ([eda32d8acf](https://github.com/Devolutions/IronRDP/commit/eda32d8acffbb2e37d13c790105ff022067f5efb)) 

- Skip credential check when server credentials are None ([#1150](https://github.com/Devolutions/IronRDP/issues/1150)) ([84015c9467](https://github.com/Devolutions/IronRDP/commit/84015c946731579dfd7a49294b2e55259e4f8d3f)) 

### <!-- 7 -->Build

- Upgrade sspi to 0.19, picky to rc.22, fix NTLM fallback ([#1188](https://github.com/Devolutions/IronRDP/issues/1188)) ([c70d38a9f1](https://github.com/Devolutions/IronRDP/commit/c70d38a9f190d6ad6c84bd9027a388b5db3296ba)) 


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.7.0...ironrdp-acceptor-v0.8.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- [**breaking**] Use static dispatch for NetworkClient trait ([#1043](https://github.com/Devolutions/IronRDP/issues/1043)) ([bca6d190a8](https://github.com/Devolutions/IronRDP/commit/bca6d190a870708468534d224ff225a658767a9a)) 

  - Rename `AsyncNetworkClient` to `NetworkClient`
  - Replace dynamic dispatch (`Option<&mut dyn ...>`) with static dispatch
  using generics (`&mut N where N: NetworkClient`)
  - Reorder `connect_finalize` parameters for consistency across crates

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.5.0...ironrdp-acceptor-v0.6.0)] - 2025-07-08

### <!-- 1 -->Features

- [**breaking**] Support for server-side Kerberos (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.4.0...ironrdp-acceptor-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make the CredsspSequence type public ([5abd9ff8e0](https://github.com/Devolutions/IronRDP/commit/5abd9ff8e0da8ea48c6747526c4b703a39bf4972)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.1...ironrdp-acceptor-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.3.0...ironrdp-acceptor-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.1...ironrdp-acceptor-v0.3.0)] - 2025-01-28

### <!-- 0 -->Security

- Allow using basic RDP/no security ([7c72a9f9bb](https://github.com/Devolutions/IronRDP/commit/7c72a9f9bbe726d6f9f2377c19e9a672d8d086d5)) 

### <!-- 4 -->Bug Fixes

- Drop unexpected PDUs during deactivation-reactivation ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

  The current behavior of handling unmatched PDUs in fn read_by_hint()
  isn't good enough. An unexpected PDUs may be received and fail to be
  decoded during Acceptor::step().
  
  Change the code to simply drop unexpected PDUs (as opposed to attempting
  to replay the unmatched leftover, which isn't clearly needed)

- Reattach existing channels ([c4587b537c](https://github.com/Devolutions/IronRDP/commit/c4587b537c7c0a148e11bc365bc3df88e2c92312)) 

  I couldn't find any explicit behaviour described in the specification,
  but apparently, we must just keep the channel state as they were during
  reactivation. This fixes various state issues during client resize.

- Do not restart static channels on reactivation ([82c7c2f5b0](https://github.com/Devolutions/IronRDP/commit/82c7c2f5b08c44b1a4f6b04c13ad24d9e2ffa371)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-acceptor-v0.2.0...ironrdp-acceptor-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
