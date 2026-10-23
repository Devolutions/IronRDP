# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.9.0...ironrdp-pdu-v0.10.0)] - 2026-10-23

### <!-- 0 -->Security

- Redact the auto-reconnect random and the raw auto-reconnect cookie from the `Debug` output of `ServerAutoReconnect`, `LogonInfoExtended` and `ExtendedClientOptionalInfo` ([#1496](https://github.com/Devolutions/IronRDP/issues/1496)) ([d0948faa18](https://github.com/Devolutions/IronRDP/commit/d0948faa187da673e9ded44e9e22cfbefc2c7f62))

### <!-- 1 -->Features

- Add `ClientAutoReconnect` for the Client Auto-Reconnect Packet, exposed through `ExtendedClientOptionalInfo::auto_reconnect`, with `from_server_cookie`, `verify`, `to_bytes`, `Encode`, `Decode` and a `Debug` output that omits the security verifier ([#1509](https://github.com/Devolutions/IronRDP/issues/1509)) ([44f675e244](https://github.com/Devolutions/IronRDP/commit/44f675e244ee76b5311756668ffbbe28e98c7175)) ([#1501](https://github.com/Devolutions/IronRDP/issues/1501)) ([74b3365c1f](https://github.com/Devolutions/IronRDP/commit/74b3365c1f98c0da6feed7507779c67e1b8e6d08)) ([#1496](https://github.com/Devolutions/IronRDP/issues/1496)) ([d0948faa18](https://github.com/Devolutions/IronRDP/commit/d0948faa187da673e9ded44e9e22cfbefc2c7f62))

- Add the `rdp::server_redirection` module with `ServerRedirectionPdu`, its Standard and Enhanced Security framings, and `decode_server_redirection` ([#2208](https://github.com/Devolutions/IronRDP/issues/2208)) ([485657a6ec](https://github.com/Devolutions/IronRDP/commit/485657a6eca682a0356e100a9fda229342861fd7))

- [**breaking**] Add `IoChannelPdu::ServerRedirection` and `ShareControlPdu::ServerRedirect` for the Server Redirection PDU ([#2208](https://github.com/Devolutions/IronRDP/issues/2208)) ([485657a6ec](https://github.com/Devolutions/IronRDP/commit/485657a6eca682a0356e100a9fda229342861fd7))

- [**breaking**] Add the `ClientInfo::password_cookie` field to carry a redirection password cookie, decoded when the `Password` field is not valid text ([#2208](https://github.com/Devolutions/IronRDP/issues/2208)) ([485657a6ec](https://github.com/Devolutions/IronRDP/commit/485657a6eca682a0356e100a9fda229342861fd7))

- [**breaking**] Add the `ConnectionRequest::correlation_info` field and the `CorrelationInfo` type to encode and decode `RDP_NEG_CORRELATION_INFO` ([#1582](https://github.com/Devolutions/IronRDP/issues/1582)) ([c4483617ba](https://github.com/Devolutions/IronRDP/commit/c4483617ba05c31182b58c58be66bd41120a076d))

- Add `OpaqueRoutingToken`, `ConnectionRequestWithOpaqueRoutingToken` and `MAX_ROUTING_TOKEN_LENGTH` for X.224 Connection Requests carrying an opaque routing token ([#1781](https://github.com/Devolutions/IronRDP/issues/1781)) ([14ef4fd49f](https://github.com/Devolutions/IronRDP/commit/14ef4fd49fcf950169806866ee67db0c49662cfc))

- Add `HeartbeatPdu` for the Server Heartbeat PDU ([#1814](https://github.com/Devolutions/IronRDP/issues/1814)) ([4275b3d7fd](https://github.com/Devolutions/IronRDP/commit/4275b3d7fdf33c0aedeeb6bc9f58bfec83d0893d))

- Add `ClientMessageChannelPdu` to decode the Auto-Detect Response and Initiate Multitransport Response PDUs from the message channel ([#1964](https://github.com/Devolutions/IronRDP/issues/1964)) ([6aebe6d0b1](https://github.com/Devolutions/IronRDP/commit/6aebe6d0b11c4aa3c99252bd986e138e43756a6d))

- Add the `window` module, which decodes windowing orders from slow-path and Fast-Path Orders updates into typed window, notification icon and desktop orders, and encodes them as a slow-path Orders update ([#2195](https://github.com/Devolutions/IronRDP/issues/2195)) ([2bc793ef3f](https://github.com/Devolutions/IronRDP/commit/2bc793ef3f5bb21b4a219175e653a2f65d6f5dac)) ([#2210](https://github.com/Devolutions/IronRDP/issues/2210)) ([f5ec943588](https://github.com/Devolutions/IronRDP/commit/f5ec943588a66afada084527a113ce1466b13d31))

- [**breaking**] Add `FastPathUpdate::Orders` carrying raw Fast-Path Orders update data ([#1631](https://github.com/Devolutions/IronRDP/issues/1631)) ([0c3fbe78b4](https://github.com/Devolutions/IronRDP/commit/0c3fbe78b4366533b9fcea046b2b53654e003a72))

- [**breaking**] Change `CapabilitySet::WindowList` to hold the typed `WindowList` capability with `WindowSupportLevel` ([#1631](https://github.com/Devolutions/IronRDP/issues/1631)) ([0c3fbe78b4](https://github.com/Devolutions/IronRDP/commit/0c3fbe78b4366533b9fcea046b2b53654e003a72))

- [**breaking**] Change `CapabilitySet::Rail` to hold the typed `Rail` capability with `RailSupportLevel` flags ([#1636](https://github.com/Devolutions/IronRDP/issues/1636)) ([0161906731](https://github.com/Devolutions/IronRDP/commit/0161906731757356953cdb389a2cd6a42863deb2))

- [**breaking**] Add `ShareDataPdu::Compressed`, which holds a bulk-compressed Share Data PDU body undecoded, and `ShareDataPdu::decode_with_type` to decode it once decompressed ([#1518](https://github.com/Devolutions/IronRDP/issues/1518)) ([6151e21bf5](https://github.com/Devolutions/IronRDP/commit/6151e21bf58b7297e9b4abc2167aa36fc2ba77e4))

- [**breaking**] Add the `compression_flags` and `compression_type` fields to `ShareDataCtx` ([#1518](https://github.com/Devolutions/IronRDP/issues/1518)) ([6151e21bf5](https://github.com/Devolutions/IronRDP/commit/6151e21bf58b7297e9b4abc2167aa36fc2ba77e4))

- Add `describe_unexpected_share_control_pdu` to name an unexpected Share Control PDU ([#1329](https://github.com/Devolutions/IronRDP/issues/1329)) ([df1f7e7faa](https://github.com/Devolutions/IronRDP/commit/df1f7e7faaf068435bfbbe1efcb4a8800ebb3d9f))

- Add the `FastPathInput::MAX_EVENTS`, `MousePdu::WHEEL_ROTATION_RANGE` and `TILE_FLAG_DIFFERENCE` constants ([#1630](https://github.com/Devolutions/IronRDP/issues/1630)) ([3818b48037](https://github.com/Devolutions/IronRDP/commit/3818b480375ec9411d5319d8e7af161f1d662cbf)) ([#1818](https://github.com/Devolutions/IronRDP/issues/1818)) ([2685d85cc4](https://github.com/Devolutions/IronRDP/commit/2685d85cc4f95e2ee3294e4edda4d0ff41f9ff80)) ([#1698](https://github.com/Devolutions/IronRDP/issues/1698)) ([69e323ae47](https://github.com/Devolutions/IronRDP/commit/69e323ae473264e13b2bb3f70a356cd967debea8))

- Report the byte offset at which decoding or encoding failed in PDU errors ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

- [**breaking**] Make `utils::encode_string` take a `&mut WriteCursor<'_>` instead of a `&mut [u8]` ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec))

### <!-- 4 -->Bug Fixes

- [**breaking**] Make `KeyboardType` a `u32` newtype with associated constants, adding `KOREAN`, so any Client Core Data `keyboardType` value decodes and round-trips, and decode a zero `keyboardType` in the Input capability set as `None` ([#1689](https://github.com/Devolutions/IronRDP/issues/1689)) ([c74e7c5c94](https://github.com/Devolutions/IronRDP/commit/c74e7c5c9431c87e57a1550b037867d235c1d362))

- [**breaking**] Add the `ServerShutdown` and `ServerReboot` error info codes to `ProtocolIndependentCode` ([#1966](https://github.com/Devolutions/IronRDP/issues/1966)) ([d2bb737664](https://github.com/Devolutions/IronRDP/commit/d2bb7376642c8b03178226e1cc0f87698d511cfb))

- Skip unknown GCC user-data blocks instead of failing the settings exchange, and add `UserDataHeader::decode_raw` ([#1489](https://github.com/Devolutions/IronRDP/issues/1489)) ([629a8024f4](https://github.com/Devolutions/IronRDP/commit/629a8024f4832ed04247ef56597604bbb4b85017))

- Tolerate unknown flag bits instead of rejecting the PDU in `BasicSecurityHeader`, `ClientInfoFlags`, the GCC `RedirectionFlags`, `MonitorFlags`, `MultiTransportFlags`, `encryptionMethods`, `serverSelectedProtocol` and `clientReqProtocols` fields, Fast-Path keyboard and synchronize input events, and `ChannelOptions` ([#1458](https://github.com/Devolutions/IronRDP/issues/1458)) ([a4acab488b](https://github.com/Devolutions/IronRDP/commit/a4acab488b0d549854ebe0e0e922fe7252e84c98)) ([#1843](https://github.com/Devolutions/IronRDP/issues/1843)) ([ff48d8a8e9](https://github.com/Devolutions/IronRDP/commit/ff48d8a8e93408f10c511dbaf916d38921fc6fcf)) ([#1844](https://github.com/Devolutions/IronRDP/issues/1844)) ([e8a75797e7](https://github.com/Devolutions/IronRDP/commit/e8a75797e7ce3be1682e9a5d1810bc0102662440)) ([#1845](https://github.com/Devolutions/IronRDP/issues/1845)) ([f87040f2c3](https://github.com/Devolutions/IronRDP/commit/f87040f2c33f9273690f91e85ac23f3ce0e7f1a7)) ([#1846](https://github.com/Devolutions/IronRDP/issues/1846)) ([9af99317c2](https://github.com/Devolutions/IronRDP/commit/9af99317c217f4fcc38fa381d28e30d9ecfc5be5)) ([#1847](https://github.com/Devolutions/IronRDP/issues/1847)) ([0c3fc7027c](https://github.com/Devolutions/IronRDP/commit/0c3fc7027cd02a49f892379ced124304b84ee2c9)) ([#1911](https://github.com/Devolutions/IronRDP/issues/1911)) ([e56bb42cca](https://github.com/Devolutions/IronRDP/commit/e56bb42cca9b5a5d5afdef353e502ab078ef24b7))

- Accept the `SEC_RESET_SEQNO` and `SEC_IGNORE_SEQNO` flags on Initiate Multitransport Request and Response PDUs, and reject other PDU-type flags ([#1098](https://github.com/Devolutions/IronRDP/issues/1098)) ([e45fbfe0f5](https://github.com/Devolutions/IronRDP/commit/e45fbfe0f597011706e77fc174ca14e5e9d435b9))

- Accept a Share Control Header without `shareId` (xrdp), an under-declared `totalLength` on a Server Font Map PDU (VirtualBox), and a zero `totalLength` on empty Update and Pointer PDUs ([#1719](https://github.com/Devolutions/IronRDP/issues/1719)) ([d4b728a1d4](https://github.com/Devolutions/IronRDP/commit/d4b728a1d40b917dd94da6d2f8f12a32ce0965ec)) ([#1541](https://github.com/Devolutions/IronRDP/issues/1541)) ([9b04c90ca2](https://github.com/Devolutions/IronRDP/commit/9b04c90ca204ac8dc861cc743881df3f6b6eb501)) ([#1515](https://github.com/Devolutions/IronRDP/issues/1515)) ([33506e6139](https://github.com/Devolutions/IronRDP/commit/33506e613923dae504f46451231dcee15a6320a2))

- Decode an empty Font Map PDU body as the default `FontPdu` ([#1506](https://github.com/Devolutions/IronRDP/issues/1506)) ([e496b7b8ea](https://github.com/Devolutions/IronRDP/commit/e496b7b8eaf60688fc0d507961713c6aabd0e05e))

- Reject Fast-Path frames whose length is smaller than the header, and TPKT headers shorter than 7 bytes ([#1515](https://github.com/Devolutions/IronRDP/issues/1515)) ([33506e6139](https://github.com/Devolutions/IronRDP/commit/33506e613923dae504f46451231dcee15a6320a2))

- Derive the optional fields of `AutoDetectRequest::NetworkCharacteristicsResult` and the connect-time `BandwidthMeasureStop` payload from `requestType` when encoding, and fail when a required value is missing ([#1491](https://github.com/Devolutions/IronRDP/issues/1491)) ([f9cc62fa2c](https://github.com/Devolutions/IronRDP/commit/f9cc62fa2ccb4f211838dd0961c19cdf3b79a38e))

- Count UTF-16 code units for the v2 payload length of `PreconnectionBlob` ([#1505](https://github.com/Devolutions/IronRDP/issues/1505)) ([5c1816244e](https://github.com/Devolutions/IronRDP/commit/5c1816244e83187a04249e9d9c240d096cb78f55))

- Make `MONITOR_COUNT_MAX` public, and return an error when encoding `ClientMonitorData` with more monitors instead of truncating the list ([#1675](https://github.com/Devolutions/IronRDP/issues/1675)) ([063efcdc30](https://github.com/Devolutions/IronRDP/commit/063efcdc3088d8f44e423cc322077d40bf9aadf2))

- Add `Quant::try_new`, `Quant::validate` and `Quant::VALID_RANGE`, and return an error when encoding a `Quant` with an out-of-range value instead of panicking ([#1685](https://github.com/Devolutions/IronRDP/issues/1685)) ([925e7c0f7c](https://github.com/Devolutions/IronRDP/commit/925e7c0f7cb7ae937a92ec93d4cb758289594cc0))

- Read the ClearCodec short V-Bar `yOn` and `yOff` fields from the documented bit positions ([#1694](https://github.com/Devolutions/IronRDP/issues/1694)) ([c1db8608af](https://github.com/Devolutions/IronRDP/commit/c1db8608af35e3c1318a50fa812f3259ff69d800))

- Decode the packed byte of ClearCodec RLEX segments when the palette has a single entry, and reject segments whose stop index exceeds the palette or whose suite depth exceeds the stop index ([#1903](https://github.com/Devolutions/IronRDP/issues/1903)) ([8e76c3168d](https://github.com/Devolutions/IronRDP/commit/8e76c3168da578826d8fd707503655ea16bd3392)) ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa))

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `x509-cert` to 0.3, which changes the error type in `ServerLicenseError::InvalidX509Certificate` ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b))

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.8.0...ironrdp-pdu-v0.9.0)] - 2026-07-10

### <!-- 0 -->Security

- [**breaking**] Send NetworkAutoDetect over the MCS message channel ([#1348](https://github.com/Devolutions/IronRDP/issues/1348)) ([8a1fd0118e](https://github.com/Devolutions/IronRDP/commit/8a1fd0118e0bac214c9050b6ca6b36a040046dd3)) 

  Corrects Network Auto-Detect framing and routing to match MS-RDPBCGR by
  moving it off the I/O channel slow-path Share Data PDUs and onto the MCS
  message channel with the required Basic Security Header
  (SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP). This aligns IronRDP with
  mstsc/xfreerdp behavior and enables both connect-time and continuous
  auto-detection to actually function.

### <!-- 4 -->Bug Fixes

- Set COMPRESSION_USED on the FastPath update header when compressed ([#1382](https://github.com/Devolutions/IronRDP/issues/1382)) ([3f96d0029d](https://github.com/Devolutions/IronRDP/commit/3f96d0029d37d3cee84b419bbf4d53b5519e385d)) 

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

- Adopt MCS and RDP header utilities relocated from ironrdp-connector ([#1419](https://github.com/Devolutions/IronRDP/issues/1419)) ([5c22f86a71](https://github.com/Devolutions/IronRDP/commit/5c22f86a7150bc10c26a3be39bfaebf84c67d781)) 

  Hosts the shared MCS and RDP security-header helpers previously living in ironrdp-connector's legacy modules.

- Decode MousePdu wheel rotation as two's complement, matching encode ([#1415](https://github.com/Devolutions/IronRDP/issues/1415)) ([9b4d01b403](https://github.com/Devolutions/IronRDP/commit/9b4d01b4038ede1cdd329fd9ea47a5d241480d1d)) 



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.7.0...ironrdp-pdu-v0.8.0)] - 2026-05-27

### <!-- 1 -->Features

- Add Initiate Multitransport Request/Response PDU types ([#1091](https://github.com/Devolutions/IronRDP/issues/1091)) ([5a50f4099b](https://github.com/Devolutions/IronRDP/commit/5a50f4099b8f8173c5c067089a0d372402dbb52d)) 

  Add MultitransportRequestPdu and MultitransportResponsePdu types for the
  sideband UDP transport bootstrapping PDUs defined in MS-RDPBCGR
  2.2.15.1 and 2.2.15.2. Needed to decode/encode the IO channel messages that
  initiate UDP transport setup.

- Add Auto-Detect Request and Response PDU types ([#1168](https://github.com/Devolutions/IronRDP/issues/1168)) ([6e5f08a1b9](https://github.com/Devolutions/IronRDP/commit/6e5f08a1b95f69b9d8182a75298b74aaf829ac39)) 

- [**breaking**] Route auto-detect PDUs through ShareDataPdu dispatch ([#1176](https://github.com/Devolutions/IronRDP/issues/1176)) ([e5f2f36e96](https://github.com/Devolutions/IronRDP/commit/e5f2f36e96dfb2036236c99a1ee83c5a36bf281f)) 

  Added Share Data PDU dispatch support for auto-detect PDUs, improving compatibility with Windows servers.

- Complete pixel format support for bitmap updates ([#1134](https://github.com/Devolutions/IronRDP/issues/1134)) ([a6b41093ce](https://github.com/Devolutions/IronRDP/commit/a6b41093ce4ece081d2538c157f6bc547c3b2607)) 

  Wires missing bitmap pixel formats (8/15/24bpp) into the session rendering
  pipeline so bitmap updates at those depths are rendered instead of being
  dropped, and adds fast-path palette update parsing to support 8bpp indexed
  color sessions.

- Add RemoteFX Progressive codec primitives ([#1196](https://github.com/Devolutions/IronRDP/issues/1196)) ([49099f0c31](https://github.com/Devolutions/IronRDP/commit/49099f0c3136c25b67801fb1b07f78542dc796de)) 

  Add wire-format types for RemoteFX Progressive Codec (MS-RDPRFX
  Progressive Extension) and the computational primitives required for progressive refinement.

- Handle slow-path graphics and pointer updates ([#1132](https://github.com/Devolutions/IronRDP/issues/1132)) ([9383380292](https://github.com/Devolutions/IronRDP/commit/938338029290f1be82a7f784d544bb77ac797aeb)) 

  Adds support for slow-path graphics and pointer updates to IronRDP, fixing connectivity issues with servers like XRDP that use slow-path output instead of fast-path. The implementation parses slow-path framing headers and routes the inner payload structures through the existing fast-path processing pipeline by extracting shared bitmap and pointer processing methods.

- Add progressive RFX decode and EGFX integration ([#1197](https://github.com/Devolutions/IronRDP/issues/1197)) ([a142799d1d](https://github.com/Devolutions/IronRDP/commit/a142799d1dcbdcd6546ec6e75173fbfe66f0ea67)) 

- Add ClearCodec bitmap compression codec ([#1174](https://github.com/Devolutions/IronRDP/issues/1174)) ([059ca902a5](https://github.com/Devolutions/IronRDP/commit/059ca902a5518113163042225bc5d2088869933a)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Remove unused legacy error types ([#1268](https://github.com/Devolutions/IronRDP/issues/1268)) ([df0bf9c69d](https://github.com/Devolutions/IronRDP/commit/df0bf9c69d88febaf6b82c479fdc7dcafe226567)) 

  Remove GccError, McsError, RdpError, SecurityDataError,
  ClusterDataError, NetworkDataError, CoreDataError, InputEventError,
  ClientInfoError, CapabilitySetsError, SessionError, and ChannelError.
  All encode/decode functions had already been migrated to use
  DecodeResult/EncodeResult from ironrdp-core, leaving these error types
  as dead code.

- Accept short Server Deactivate All PDU ([485d6c2f8d](https://github.com/Devolutions/IronRDP/commit/485d6c2f8d6f95bb06ca14cbfa4c56a27abbad0e)) 

  Some servers (XRDP, older Windows) send a Deactivate All PDU without
  the sourceDescriptor field. The decode previously required at least 3
  bytes, which caused a hard failure during deactivation-reactivation
  sequences with these servers.
  
  Treat the sourceDescriptor as optional: if the remaining data is
  shorter than the fixed part size, return successfully without
  reading the field. FreeRDP handles this the same way.

- Correct ShareDataHeader uncompressedLength calculation ([#1148](https://github.com/Devolutions/IronRDP/issues/1148)) ([c2688f464d](https://github.com/Devolutions/IronRDP/commit/c2688f464d8cbf239d35e5b43538195b1870eed8)) 

- Replace all from_bits_truncate with from_bits_retain ([#1144](https://github.com/Devolutions/IronRDP/issues/1144)) ([353e30ddfd](https://github.com/Devolutions/IronRDP/commit/353e30ddfdaafc897db10b8663e364ef7775a7fd)) 

  from_bits_truncate silently discards unknown bits, which breaks the
  encode/decode round-trip property. This matters for fuzzing because a
  PDU that decodes and re-encodes should produce identical bytes.
  from_bits_retain preserves all bits, including those not yet defined in
  our bitflags types, so the round-trip property holds.

- [**breaking**] Remove ironrdp-egfx duplicates from ironrdp-pdu ([#1303](https://github.com/Devolutions/IronRDP/issues/1303)) ([491b91fd2f](https://github.com/Devolutions/IronRDP/commit/491b91fd2f33235e4b31dea5c4a215e67f734179)) 

- Cover BitmapCacheV3 in CapabilitySet encoder ([#1313](https://github.com/Devolutions/IronRDP/issues/1313)) ([a71567e35e](https://github.com/Devolutions/IronRDP/commit/a71567e35e47a6eba8493c00933e0b66e0c63d5b)) 

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.5.0...ironrdp-pdu-v0.6.0)] - 2025-08-29

### <!-- 1 -->Features

- Implement `Default` trait on `ExtendedClientOptionalInfoBuilder` (#891) ([ae052ed835](https://github.com/Devolutions/IronRDP/commit/ae052ed83598ad1f4ad7038b153e3c5398d2a738)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Update timezone info to use i32 bias (#921) ([119c7077c9](https://github.com/Devolutions/IronRDP/commit/119c7077c98e4b43021619378c4f251c1f95ae17)) 

  Switches `bias` from an unsigned to a signed integer.
  This matches the updated specification from Microsoft.

### <!-- 7 -->Build

- Bump thiserror to 2.0 ([b4fb0aa0c7](https://github.com/Devolutions/IronRDP/commit/b4fb0aa0c79aa409d1b6a5f43ab23448eede4e51)) 

- Bump der-parser to 10.0 ([03cac54ada](https://github.com/Devolutions/IronRDP/commit/03cac54ada50fae13d085b855a9b8db37d615ba8)) 

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.4.0...ironrdp-pdu-v0.5.0)] - 2025-05-27

### <!-- 1 -->Features

- Make client_codecs_capabilities() configurable ([783702962a](https://github.com/Devolutions/IronRDP/commit/783702962a2e842f9d5046ac706048ba124e1401)) 

- BitmapCodecs struct ([f03ee393a3](https://github.com/Devolutions/IronRDP/commit/f03ee393a36906114b5bcba0e88ebc6869a99785)) 

### <!-- 4 -->Bug Fixes

- Fix possible out of bound indexing in RFX module (#724) ([9f4e6d410b](https://github.com/Devolutions/IronRDP/commit/9f4e6d410b631d8a6b0c09c2abc0817a83cf042b)) 

  An index bound check was missing in the RFX module. Found by fuzzer.

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.1...ironrdp-pdu-v0.4.0)] - 2025-03-12

### <!-- 4 -->Bug Fixes

- TS_RFX_CHANNELT width/height SHOULD be within range ([097cdb66f9](https://github.com/Devolutions/IronRDP/commit/097cdb66f965700caeea5659ff7fe4a129b84838)) 

  According to the specification, the value does not need to be in the range:
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/4060f07e-9d73-454d-841e-131a93aca675
  
  (the ironrdp-server can send larger values)

### Refactor

- [**breaking**] Remove RfxChannelWidth and RfxChannelHeight structs ([7cb1ac99d1](https://github.com/Devolutions/IronRDP/commit/7cb1ac99d189cdcaa17fa17e51f95be630e9982e)) 

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.3.0...ironrdp-pdu-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.2.0...ironrdp-pdu-v0.3.0)] - 2025-03-07

### <!-- 4 -->Bug Fixes

- Make AddressFamily parsing resilient (#672) ([6b4af94071](https://github.com/Devolutions/IronRDP/commit/6b4af94071bfb0adff482cc33b75e6c37ff6e10f)) 

- Fix FastPathHeader minimal size (#687) ([3b9d558e9c](https://github.com/Devolutions/IronRDP/commit/3b9d558e9c958297d9654861df515e2a8658bf8b)) 

  The minimal_size() logic didn't properly take into account the overall
  PDU size.
  
  This fixes random error/disconnect in client.

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.2...ironrdp-pdu-v0.2.0)] - 2025-01-28

### <!-- 1 -->Features

- ClientLicenseInfo and other license PDU-related adjustments (#634) ([dd221bf224](https://github.com/Devolutions/IronRDP/commit/dd221bf22401c4635798ec012724cba7e6d503b2)) 

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-pdu-v0.1.1...ironrdp-pdu-v0.1.2)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
