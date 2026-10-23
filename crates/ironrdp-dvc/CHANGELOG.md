# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.8.0...ironrdp-dvc-v0.9.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Add the `SoftSyncRequest` and `SoftSyncResponse` variants to `DrdynvcServerPdu` and `DrdynvcClientPdu`, and mark `DrdynvcDataPdu`, `DrdynvcClientPdu` and `DrdynvcServerPdu` `#[non_exhaustive]` ([#1584](https://github.com/Devolutions/IronRDP/issues/1584)) ([bd630842ba](https://github.com/Devolutions/IronRDP/commit/bd630842bacc49cc129e613610482023a0f760db))

- Add the Soft-Sync PDUs `SoftSyncRequestPdu`, `SoftSyncResponsePdu`, `SoftSyncChannelList` and `SoftSyncTunnelType`, and client-side Soft-Sync in `DrdynvcClient` (`enable_soft_sync_tunnel`, `tunnel_for_channel`, `switched_to_tunnel`, and `process_tunnel`, which returns a `DvcMessageBatch` tagged with its channel ID) ([#1584](https://github.com/Devolutions/IronRDP/issues/1584)) ([bd630842ba](https://github.com/Devolutions/IronRDP/commit/bd630842bacc49cc129e613610482023a0f760db)) ([#1826](https://github.com/Devolutions/IronRDP/issues/1826)) ([a989229409](https://github.com/Devolutions/IronRDP/commit/a9892294095d32808ee24dc8dad74dce46cb0bb4)) ([#2008](https://github.com/Devolutions/IronRDP/issues/2008)) ([bfd16a2e55](https://github.com/Devolutions/IronRDP/commit/bfd16a2e5573d4cd6617dd71b2802b4e7b944a59))

- Add Soft-Sync on the server: `DrdynvcServer::request_reliable_udp`, `tunnel_for_outgoing_channel`, `soft_sync_awaiting_response`, `soft_sync_response_received` and `process_tunnel` ([#1584](https://github.com/Devolutions/IronRDP/issues/1584)) ([bd630842ba](https://github.com/Devolutions/IronRDP/commit/bd630842bacc49cc129e613610482023a0f760db)) ([#1954](https://github.com/Devolutions/IronRDP/issues/1954)) ([73dfa30e38](https://github.com/Devolutions/IronRDP/commit/73dfa30e3831e51cf8aa58f93e27b1b00102a7ec))

- [**breaking**] Replace `DrdynvcClient::get_dvc_by_type_id` with `DrdynvcClient::get_dvc` and `get_dvc_mut`, which return a typed `DynamicChannelRef` or `DynamicChannelMut` ([#1377](https://github.com/Devolutions/IronRDP/issues/1377)) ([d43ecf9a54](https://github.com/Devolutions/IronRDP/commit/d43ecf9a54363d37e0c485a1e9e73da0d47ae540)) ([#1778](https://github.com/Devolutions/IronRDP/issues/1778)) ([1cee7a8613](https://github.com/Devolutions/IronRDP/commit/1cee7a86135a0556c01965d0406233bd7df367a9))

- [**breaking**] Make `DrdynvcClient::get_dvc_by_channel_id` and `get_dvc_by_channel_id_mut` generic over the processor type, returning `DynamicChannelRef` and `DynamicChannelMut` ([#1377](https://github.com/Devolutions/IronRDP/issues/1377)) ([d43ecf9a54](https://github.com/Devolutions/IronRDP/commit/d43ecf9a54363d37e0c485a1e9e73da0d47ae540))

- [**breaking**] Remove `DynamicVirtualChannel` from the public API ([#1377](https://github.com/Devolutions/IronRDP/issues/1377)) ([d43ecf9a54](https://github.com/Devolutions/IronRDP/commit/d43ecf9a54363d37e0c485a1e9e73da0d47ae540))

- [**breaking**] Return `Box<dyn DvcClientProcessor>` instead of `Box<dyn DvcProcessor>` from `DvcChannelListener::create` ([#1377](https://github.com/Devolutions/IronRDP/issues/1377)) ([d43ecf9a54](https://github.com/Devolutions/IronRDP/commit/d43ecf9a54363d37e0c485a1e9e73da0d47ae540))

- Add `DrdynvcClient::has_registered_dvc` ([#1522](https://github.com/Devolutions/IronRDP/issues/1522)) ([57b1366650](https://github.com/Devolutions/IronRDP/commit/57b13666506dc40c15b4c4702d35150beee99133))

- Add `DvcChannelListener::is_available`, with a default implementation, so a listener can report that it can no longer create a channel ([#1522](https://github.com/Devolutions/IronRDP/issues/1522)) ([57b1366650](https://github.com/Devolutions/IronRDP/commit/57b13666506dc40c15b4c4702d35150beee99133))

- Add `DrdynvcClient::with_dynamic_channel_factory` to register a channel whose processor is recreated each time the server opens it ([#2184](https://github.com/Devolutions/IronRDP/issues/2184)) ([0f735b174a](https://github.com/Devolutions/IronRDP/commit/0f735b174a9f48e71c6857d80f30d6c3ca60555b))

- Add `DrdynvcClient::attach_established_dynamic_channel` to attach a processor to an already known channel ID, for offline replay ([#1664](https://github.com/Devolutions/IronRDP/issues/1664)) ([93780feeec](https://github.com/Devolutions/IronRDP/commit/93780feeec1f09e13c8dd4691d5c5da20fae9310))

- Add `DrdynvcServer::create_channel_with` to build a processor with the channel ID reserved for it ([#1416](https://github.com/Devolutions/IronRDP/issues/1416)) ([41293c2442](https://github.com/Devolutions/IronRDP/commit/41293c2442dfb2da6b61ca05a0c842706d048fc1))

### <!-- 4 -->Bug Fixes

- Decode the `DYNVC_DATA_FIRST_COMPRESSED` and `DYNVC_DATA_COMPRESSED` PDUs of DRDYNVC version 3, which the client accepts when the server offers that version, on both client and server, through the new `DataFirstCompressedPdu` and `DataCompressedPdu` and the matching `DrdynvcDataPdu` variants ([#2173](https://github.com/Devolutions/IronRDP/issues/2173)) ([c3dd17e607](https://github.com/Devolutions/IronRDP/commit/c3dd17e6071342aa5f92b743a89a38382665eb64))

- Return an error, instead of logging and dropping the message, when a data PDU makes a message longer than its declared length ([#2173](https://github.com/Devolutions/IronRDP/issues/2173)) ([c3dd17e607](https://github.com/Devolutions/IronRDP/commit/c3dd17e6071342aa5f92b743a89a38382665eb64))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.7.0...ironrdp-dvc-v0.8.0)] - 2026-07-10

### <!-- 1 -->Features

- Expose dynamic channel accessors ([#1368](https://github.com/Devolutions/IronRDP/issues/1368)) ([985d353543](https://github.com/Devolutions/IronRDP/commit/985d353543cf45eacfe0cc57aca86502665a3a44)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.6.0...ironrdp-dvc-v0.7.0)] - 2026-06-05

### <!-- 4 -->Bug Fixes

- [**breaking**] Add channel_id parameter to DvcChannelListener::create ([#1358](https://github.com/Devolutions/IronRDP/issues/1358)) ([f21470c6dc](https://github.com/Devolutions/IronRDP/commit/f21470c6dc20e1b10b4bbf750a406644479a4b35)) 

  Updates the dynamic virtual channel (DVC) client listener interface in ironrdp-dvc to pass the channel_id (from the incoming DYNVC_CREATE_REQ) into the listener’s create method, enabling listeners to differentiate/control per-instance behavior based on the negotiated dynamic channel ID.



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.5.0...ironrdp-dvc-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Implement ECHO virtual channel ([#1109](https://github.com/Devolutions/IronRDP/issues/1109)) ([6f6496ad29](https://github.com/Devolutions/IronRDP/commit/6f6496ad29395099563d50417d6dfff623914ee6)) 

- Add DvcChannelListener for multi-instance DVC support ([#1142](https://github.com/Devolutions/IronRDP/issues/1142)) ([28e8628f0e](https://github.com/Devolutions/IronRDP/commit/28e8628f0e3cea9f7723a73abf5fd7ed2da968f0)) 

- Close channel API for server and client ([#1302](https://github.com/Devolutions/IronRDP/issues/1302)) ([196d18dfaa](https://github.com/Devolutions/IronRDP/commit/196d18dfaa7ec899946bb90f4dcb8bad31872f48)) 

### <!-- 4 -->Bug Fixes

- Negotiate DVC version from server capabilities ([d094cbeb75](https://github.com/Devolutions/IronRDP/commit/d094cbeb7501c83fc6ad5401ba69d22f79d6657c)) 

  The client was hardcoded to respond with CapsVersion::V1 regardless
  of what the server requested. Servers that require V2 or V3 (such
  as XRDP) would reject the channel with "Dynamic Virtual Channel
  version 1 is not supported."
  
  Echo the server's requested version in the capabilities response
  instead. This correctly handles V1, V2, and V3 depending on what
  the server advertises. When a Create arrives before Capabilities
  (fallback path), default to V2 as the most broadly compatible
  version.
  
  Also bump the server-side capabilities request from V1 to V2 to
  advertise priority charge support.
  
  Add CapabilitiesRequestPdu::version() accessor to expose the
  server's requested version from the parsed PDU.

## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.4.0...ironrdp-dvc-v0.4.1)] - 2025-09-04

### <!-- 1 -->Features

- Add API to attach dynamic channels to an already created `DrdynvcClient` instance (#938) ([17833fe009](https://github.com/Devolutions/IronRDP/commit/17833fe009279823c4076d3e2e0c7d063fd24a43)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.3.0...ironrdp-dvc-v0.3.1)] - 2025-06-27

### <!-- 1 -->Features

- Add `DynamicChannelSet::get_by_channel_id` (#791) ([5482365655](https://github.com/Devolutions/IronRDP/commit/5482365655e5c171cd967eda401b01161a9f6602)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.3...ironrdp-dvc-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.2...ironrdp-dvc-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.1...ironrdp-dvc-v0.1.2)] - 2025-01-28

### <!-- 1 -->Features

- Some debug statement on invalid channel state ([265b661b81](https://github.com/Devolutions/IronRDP/commit/265b661b81af19860c4564ba35ad22564f61cd02)) 

- Add CreationStatus::NOT_FOUND ([ab8a87d942](https://github.com/Devolutions/IronRDP/commit/ab8a87d94259a4e1df5f3a2a8d4c592377857b21)) 

  For completeness, this error is used by FreeRDP.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.1.0...ironrdp-dvc-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
