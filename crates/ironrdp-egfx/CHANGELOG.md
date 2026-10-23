# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.3.0...ironrdp-egfx-v0.4.0)] - 2026-10-23

### <!-- 1 -->Features

- Decode ClearCodec, Planar, RFX Progressive, AVC444, AVC444v2, RemoteFX and Alpha surface commands in `GraphicsPipelineClient` ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa)) ([#1507](https://github.com/Devolutions/IronRDP/issues/1507)) ([66c8a81be0](https://github.com/Devolutions/IronRDP/commit/66c8a81be0a9f966e3cf4935ca2a0274d10b063f)) ([#1673](https://github.com/Devolutions/IronRDP/issues/1673)) ([f21f1979f4](https://github.com/Devolutions/IronRDP/commit/f21f1979f4ef7ce8756a4022d866c7f7fc150b0b)) ([#2199](https://github.com/Devolutions/IronRDP/issues/2199)) ([a0c8c0dc1e](https://github.com/Devolutions/IronRDP/commit/a0c8c0dc1e1296f428092d3158585cfc7d395efb)) ([#2194](https://github.com/Devolutions/IronRDP/issues/2194)) ([52476f3cee](https://github.com/Devolutions/IronRDP/commit/52476f3cee6015ddb04a62e0377b17b33ba78e19))

- Add the `compositor` module and `GraphicsPipelineClient::drain_output`, which composite surface commands, including scaled-output mappings, into output pixel updates ([#1460](https://github.com/Devolutions/IronRDP/issues/1460)) ([9cd36952ca](https://github.com/Devolutions/IronRDP/commit/9cd36952ca18196cf72c85dc42a79d5d1f5620d5)) ([#1699](https://github.com/Devolutions/IronRDP/issues/1699)) ([96e388eef2](https://github.com/Devolutions/IronRDP/commit/96e388eef27c121b5590da7a1a3fdfb1d47d11e9))

- Add `GraphicsPipelineClient::take_output_reset`, and reject a `ResetGraphics` whose output exceeds the compositor limits through the new `GraphicsPipelineHandler::on_reset_graphics_rejected`, which has a default implementation ([#1874](https://github.com/Devolutions/IronRDP/issues/1874)) ([2b44c62bdb](https://github.com/Devolutions/IronRDP/commit/2b44c62bdbd997f4aa33bdbaf4749ac4ecc85140))

- Add a persistent bitmap cache through the `cache` module's `PersistentCacheStore` and `GraphicsPipelineClient::with_persistent_cache` ([#2194](https://github.com/Devolutions/IronRDP/issues/2194)) ([52476f3cee](https://github.com/Devolutions/IronRDP/commit/52476f3cee6015ddb04a62e0377b17b33ba78e19))

- Add `GraphicsPipelineClient::request_protocol_reset` for client-initiated protocol reset at capability versions 10.3 to 10.7 ([#2207](https://github.com/Devolutions/IronRDP/issues/2207)) ([e4a0d17b8e](https://github.com/Devolutions/IronRDP/commit/e4a0d17b8e5b1b81758202dc131d8fa2aed58a23))

- Advertise capability versions 10.7 down to 10 ahead of 8.1 and 8 in the default client capabilities, omitting sets that imply AVC444 when the H.264 decoder returns no YUV planes ([#2199](https://github.com/Devolutions/IronRDP/issues/2199)) ([a0c8c0dc1e](https://github.com/Devolutions/IronRDP/commit/a0c8c0dc1e1296f428092d3158585cfc7d395efb))

- Add `H264Decoder::decode_yuv420`, `H264Decoder::supports_yuv420` and `H264Decoder::wait_until_ready` with default implementations, `decode::Yuv420Frame`, and the `avc444` module with `Yuv444Frame` and `Avc444Layout` ([#2199](https://github.com/Devolutions/IronRDP/issues/2199)) ([a0c8c0dc1e](https://github.com/Devolutions/IronRDP/commit/a0c8c0dc1e1296f428092d3158585cfc7d395efb))

- Add `YuvMatrix` and `OpenH264Decoder::with_yuv_matrix` ([#2185](https://github.com/Devolutions/IronRDP/issues/2185)) ([ed65759d4a](https://github.com/Devolutions/IronRDP/commit/ed65759d4ac670f661fbacfb5e15d14ea98d9b38))

- Add `pdu::avc_to_annex_b`, `pdu::avc_to_annex_b_into` and `pdu::is_avc_format` ([#1326](https://github.com/Devolutions/IronRDP/issues/1326)) ([cafbef1c9f](https://github.com/Devolutions/IronRDP/commit/cafbef1c9faba78acb3e33a39556eb4c4c78c6d4)) ([#1986](https://github.com/Devolutions/IronRDP/issues/1986)) ([dadacf54b9](https://github.com/Devolutions/IronRDP/commit/dadacf54b97368816ac0ffdf68907988ee5e83a8))

- Add the `encode` module with the `H264Encoder` trait and, behind the `openh264` feature, `OpenH264Encoder` ([#1852](https://github.com/Devolutions/IronRDP/issues/1852)) ([9609c9e93b](https://github.com/Devolutions/IronRDP/commit/9609c9e93b7e33d64e9daab3326dd4caa0407532))

- Add the `send_clearcodec_frame`, `send_planar_frame`, `send_avc444v2_frame` and `try_send_uncompressed_region` senders to `GraphicsPipelineServer` ([#1175](https://github.com/Devolutions/IronRDP/issues/1175)) ([714dce4662](https://github.com/Devolutions/IronRDP/commit/714dce46627e299c57d82f4f6a5c18067a95bffa)) ([#1498](https://github.com/Devolutions/IronRDP/issues/1498)) ([409b256b41](https://github.com/Devolutions/IronRDP/commit/409b256b4168b3a63d97998da311fa9e761bef3e)) ([#1788](https://github.com/Devolutions/IronRDP/issues/1788)) ([e0727394c0](https://github.com/Devolutions/IronRDP/commit/e0727394c0c5ef172a13617594e1b2db90e006f7)) ([#2125](https://github.com/Devolutions/IronRDP/issues/2125)) ([7997e6e051](https://github.com/Devolutions/IronRDP/commit/7997e6e051b6485ff9cf1d517044302a7cb2cbd1))

- Add `try_send_avc420_frame`, `try_send_avc444_frame`, `try_send_avc444v2_frame`, `avc420_submission_state` and `avc444_submission_state` to `GraphicsPipelineServer`, which report a `FrameSubmissionError` such as `Backpressured` ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f))

- Add AVC444 and AVC444v2 tiles to mixed-codec frames with `MixedTilePayload::Avc444`, `MixedTilePayload::Avc444v2`, `Avc444Tile` and `Avc444SubStream`, and add `CodecCapabilities::avc420_in_mixed_frames` ([#2002](https://github.com/Devolutions/IronRDP/issues/2002)) ([44704e81a7](https://github.com/Devolutions/IronRDP/commit/44704e81a7a835ece00cfb5fd3c2f3609ddb2f67))

- Refuse AVC tiles in `send_mixed_frame` unless the client negotiated capability version 10.4 or later without `AVC_DISABLED`, and reject AVC420 quantization parameters above 63 when encoding ([#2002](https://github.com/Devolutions/IronRDP/issues/2002)) ([44704e81a7](https://github.com/Devolutions/IronRDP/commit/44704e81a7a835ece00cfb5fd3c2f3609ddb2f67))

- Clear server surfaces and frame tracking when a client sends `CapabilitiesAdvertise` mid-session, and add `Surfaces::reset_for_reinit` ([#1833](https://github.com/Devolutions/IronRDP/issues/1833)) ([b4ba2c2237](https://github.com/Devolutions/IronRDP/commit/b4ba2c2237a2269681256e82fc1797d22d6fe0e7))

- Send H.264 surface commands without ZGFX compression ([#2003](https://github.com/Devolutions/IronRDP/issues/2003)) ([2412bb1712](https://github.com/Devolutions/IronRDP/commit/2412bb17123632b717a8974d9627dbf31a810924))

### <!-- 4 -->Bug Fixes

- Keep the client's `AVC_DISABLED` flag in the confirmed capability set ([#1490](https://github.com/Devolutions/IronRDP/issues/1490)) ([c5bd574c8a](https://github.com/Devolutions/IronRDP/commit/c5bd574c8ac7e4ba92c31348187a196d3e84ae70))

- Declare every supported capability version in the default server preferences, so clients advertising only a mid-tier 10.x version negotiate it ([#1854](https://github.com/Devolutions/IronRDP/issues/1854)) ([8cdd788ce7](https://github.com/Devolutions/IronRDP/commit/8cdd788ce7643488ef06df743038815789fd122b))

- [**breaking**] Encode AVC420 region rectangles with exclusive right and bottom edges instead of one pixel short: `Avc420BitmapStream::rectangles` is now a `Vec<ExclusiveRectangle>` and `Avc420Region::to_rectangle` returns an `ExclusiveRectangle` ([#1788](https://github.com/Devolutions/IronRDP/issues/1788)) ([e0727394c0](https://github.com/Devolutions/IronRDP/commit/e0727394c0c5ef172a13617594e1b2db90e006f7))

- Compute the AVC444 destination rectangle from both streams ([#1788](https://github.com/Devolutions/IronRDP/issues/1788)) ([e0727394c0](https://github.com/Devolutions/IronRDP/commit/e0727394c0c5ef172a13617594e1b2db90e006f7))

- Zero `cbAvc420EncodedBitstream1` when encoding chroma-only AVC444 streams ([#1999](https://github.com/Devolutions/IronRDP/issues/1999)) ([bdf01f6b3a](https://github.com/Devolutions/IronRDP/commit/bdf01f6b3a68feb69038069999340590a5126946))

- Decode AVC420 with full-range BT.709 ([#1923](https://github.com/Devolutions/IronRDP/issues/1923)) ([9542e017da](https://github.com/Devolutions/IronRDP/commit/9542e017da4481ba4919b944f4e0742f23a83742))

- Decode AVC420 sent as an Annex B byte stream in `OpenH264Decoder` ([#1986](https://github.com/Devolutions/IronRDP/issues/1986)) ([dadacf54b9](https://github.com/Devolutions/IronRDP/commit/dadacf54b97368816ac0ffdf68907988ee5e83a8))

- Draw AVC420 partial updates from their region rectangles ([#2042](https://github.com/Devolutions/IronRDP/issues/2042)) ([#2052](https://github.com/Devolutions/IronRDP/issues/2052)) ([9f07fd4b20](https://github.com/Devolutions/IronRDP/commit/9f07fd4b2047ef6147454bace54f97bc45d2e9fb))

- Drop undecodable surface updates instead of failing the channel, and count them in `GraphicsPipelineClient::dropped_updates` ([#1789](https://github.com/Devolutions/IronRDP/issues/1789)) ([3f977f89f8](https://github.com/Devolutions/IronRDP/commit/3f977f89f87e42aa7cdc27956d30662fd90eddf3))

- Limit frames in flight to the client's Frame Acknowledge capability, with `GraphicsPipelineServer::set_client_frame_ack_limit`, `GraphicsPipelineServer::max_frames_in_flight` and `FrameTracker::max_in_flight` ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f))

- Ignore unknown and stale frame acknowledgements, and do not track frames a suspended client cannot acknowledge ([#1912](https://github.com/Devolutions/IronRDP/issues/1912)) ([6ceb5387b5](https://github.com/Devolutions/IronRDP/commit/6ceb5387b546290b5d49c60e126ad07c69fe70ba)) ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f))

- Report the client queue depth as unavailable in frame acknowledgements ([#2102](https://github.com/Devolutions/IronRDP/issues/2102)) ([e4143f2630](https://github.com/Devolutions/IronRDP/commit/e4143f26306a6639a889d80c1c484ca69c365a8f))

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-graphics` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.2.0...ironrdp-egfx-v0.3.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.8

- [**breaking**] Update `ironrdp-graphics` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-egfx-v0.1.0...ironrdp-egfx-v0.2.0)] - 2026-06-05

### <!-- 1 -->Features

- [**breaking**] Surface total_frames_decoded on the frame-ack callback ([#1345](https://github.com/Devolutions/IronRDP/issues/1345)) ([cf51bdd1d5](https://github.com/Devolutions/IronRDP/commit/cf51bdd1d5ba062132039f5ed6d7871e00af6412)) 

- Cascade Arbitrary derives across ironrdp-egfx public PDU types ([#1334](https://github.com/Devolutions/IronRDP/issues/1334)) ([479a13aa49](https://github.com/Devolutions/IronRDP/commit/479a13aa49478e333ccdc4c8fdf03aa4f36d2cac)) 

### <!-- 4 -->Bug Fixes

- [**breaking**] Make DecodedFrame fields private with getters to enforce size invariant ([#1331](https://github.com/Devolutions/IronRDP/issues/1331)) ([1534d1b40e](https://github.com/Devolutions/IronRDP/commit/1534d1b40e902a404b020fbae8e970a65ca74458)) 



## [0.1.0] - 2026-06-01

### Added

- Initial release
- MS-RDPEGFX PDU types (all 23 PDUs)
- Client-side DVC processor
- Server-side implementation with:
  - Multi-surface management (Offscreen Surfaces ADM element)
  - Frame tracking with flow control (Unacknowledged Frames ADM element)
  - V8/V8.1/V10/V10.1-V10.7 capability negotiation
  - AVC420 and AVC444 frame sending
  - QoE metrics processing
  - Cache import handling
  - Resize coordination
