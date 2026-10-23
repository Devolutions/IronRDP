# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-v0.9.0...ironrdp-dvc-v0.9.1)] - 2026-10-23

### <!-- 1 -->Features

- Redirect USB devices over URBDRC with nusb and RemoteFX USB backends ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b)) 

  Adds host-device backends for the MS-RDPEUSB (URBDRC) client and wires
  them into `ironrdp-client` and the viewer, so local USB devices can be
  redirected to the server. On Windows it redirects USB the way mstsc
  does, through the in-box RemoteFX USB drivers, when policy allows it,
  and falls back to nusb otherwise. It also fixes three interop bugs in
  `ironrdp-rdpeusb` found by redirecting a device to Windows Server 2025.
  
  ## Device coverage
  
  - **Windows, with RemoteFX USB redirection allowed**: any device,
  including HID, mass storage, smart card readers and cameras, through
  `TsUsbFlt` and `TsUsbGD`, the drivers mstsc uses. Isochronous transfers
  and the cancellation of a single request are supported. The
  `fUsbRedirectionEnableMode` policy ("Allow RDP redirection of other
  supported RemoteFX USB devices from this computer") decides who may
  claim devices: nobody by default, elevated administrators, or every
  user.
  - **Windows, otherwise**, through [nusb](https://docs.rs/nusb): only
  devices bound to WinUSB, or composite devices whose interfaces all use
  WinUSB. Firmware that requests WinUSB works out of the box; other
  devices need Zadig or similar.
  - **Linux**, through nusb: broad. Kernel drivers are detached while the
  device is redirected and reattached afterwards; the user needs udev
  access to `/dev/bus/usb`.
  - **macOS**, through nusb: only devices not claimed by a kernel or
  system extension.
  
  nusb was chosen for the portable backend because it's pure Rust and
  supports hotplug on Windows, Linux and macOS. The full list of limits is
  in `crates/ironrdp-rdpeusb-native/README.md`.
  
  ## Changes
  
  - **New crate `ironrdp-rdpeusb-native`**, publishable like
  `ironrdp-rdpeusb`. It implements `DeviceManagerBackend` and
  `UrbdrcDeviceBackend`. Both backends share device selection,
  announcement and unplug handling through a small `RedirectableDevice`
  trait.
    - **nusb backend** (all platforms):
  - lists and filters devices, opens them and claims every interface
  before announcing them to the server;
  - translates control, bulk and interrupt URBs, plus feature, status,
  descriptor, vendor/class and Microsoft OS descriptor requests;
  - handles cancel, abort pipe and clear stall, configuration and
  interface selection, port IOCTLs and device text;
  - follows plug and unplug during the session, including devices
  unplugged before their channel opens;
      - rejects isochronous transfers packet by packet;
  - bounds the data each device holds in flight (64 MiB) and each IN
  transfer (16 MiB).
  
  The translation code has no I/O and is unit-tested. Each device gets its
  own worker thread with a single-threaded Tokio runtime and one ordered
  queue per endpoint. nusb can only cancel a whole endpoint, so transfers
  cancelled only because another request was cancelled resume where they
  stopped.
  - **RemoteFX USB backend** (Windows): claims the selected devices
  through the hub filter's control object and finds them, re-enumerated
  under the generic redirection driver, as interfaces of the
  per-connection claim GUID.
  - `TS_URB`s are re-encoded and submitted as overlapped I/O on a
  completion port; the driver writes each `TS_URB_RESULT` over its
  request.
      - IO controls are forwarded, and a single request can be cancelled.
  - A device is given back to its own driver when its channel closes, when
  the server retracts it, or when the connection ends.
  
  The driver interface is undocumented and was reverse-engineered from the
  Windows 11 binaries.
  - **Device selection**: FreeRDP `/usb` rules (`id:VID:PID#...`,
  `addr:BUS:ADDR#...`, `auto`, `filter:CLASS#...` / `filter:none`) and the
  `.rdp` `usbdevicestoredirect` property (`*`, `USB\VID_xxxx&PID_yyyy...`,
  device instance IDs, `-` exclusions). `auto` and `*` keep audio, HID,
  mass storage, hubs and smart cards local by default, as FreeRDP does.
  Both backends select devices with the same rules.
  - **`ironrdp-client`**: new `usb` feature, included in `all`, that
  registers the URBDRC listener.
  - Backend events go through the unbounded ordered queue the clipboard
  already uses (`RdpInputSender::send_urbdrc`, new `RdpInputEvent::Urbdrc`
  variant). The bounded input queue drops events when full, and a lost
  completion would leave the server driver waiting forever.
  - Events are scoped to their connection and device channel, so a
  reconnection that reuses channel IDs never receives stale ones.
  - USB redirection is suppressed when the RD Gateway's device-redirection
  policy disables Plug and Play or all devices, over both the HTTP and
  RPCH transports.
  - New `ConfigBuilder::with_usb_redirection`. `usbdevicestoredirect` is
  mapped in `with_property_set` through a new `ironrdp-cfg` accessor.
  - The backend crate is re-exported as `ironrdp_client::usb`, and
  `ironrdp` gains a `client-usb` feature.
  - **`ironrdp-rdpeusb`** (client side):
  - `CANCEL_REQUEST` keeps the request pending until the backend completes
  it, so the server-side IRP finishes; FreeRDP does the same.
  - `DeviceInfo::isochronous`: `ADD_DEVICE` advertises a nonzero no-ack
  isochronous jitter buffer only when the backend performs isochronous
  transfers.
  - The listener takes the device before allocating its interface ID, so a
  refused channel doesn't use one up.
  - `TsUrbInPacket` and `TsUrbOutPacket` gain `encode_ts_urb` and
  `decode_ts_urb_result`, for backends that hand `TS_URB`s to the
  operating system as they are.
    - Interop fixes found against Windows Server 2025:
  - `REGISTER_REQUEST_CALLBACK` accepted only request completion interface
  IDs of 4 or more, but Windows picks one below 4. The device channel
  failed to decode and the whole session ended right after a device was
  announced.
  - Windows sends control requests without a data stage, including OUT
  ones such as HID `SET_IDLE`, in `TRANSFER_IN_REQUEST` messages whose
  flags keep the OUT direction. They were rejected when decoded and when
  encoded. Bulk, interrupt and isochronous transfers are still checked.
  - **`ironrdp-dvc`**: a client-initiated `DYNVC_CLOSE` now tolerates data
  that races it, with bounded bookkeeping that keeps the close order and
  lets a channel ID be reused. `close_channel_batch` returns the Soft-Sync
  tunnel the channel used, so the close travels on it.
  - **Viewer**: `--usb RULES`, repeatable, overrides the `.rdp` value.
  `--usb list` prints the local devices with the values the rules match
  against and, on Windows, each device's driver.
  - **ActiveX**: `RedirectDevices` is left as `E_NOTIMPL`. It switches
  Plug and Play device redirection (MS-RDPEPNP), not URBDRC.
  
  ## Testing
  
  - `cargo xtask check fmt -v`, `cargo xtask check lints -v` and `cargo
  xtask check locks -v` pass.
  - Unit and integration tests pass:
    - `ironrdp-rdpeusb-native` (57);
    - `ironrdp-client` with all features (68);
    - the rdpeusb/dvc suites in `ironrdp-testsuite-core` (144);
    - the client config suite in `ironrdp-testsuite-extra` (70).
  
  They cover URB translation, cancellation and resumed transfers,
  select-configuration results, completion status and HRESULT, IOCTLs,
  filter and `.rdp` parsing, the config and gateway policy mapping, the
  RemoteFX USB request buffers and status mapping, and regression tests
  for each interop fix.
  - On Windows, `--usb list` enumerates the local devices.
  - **Live, against Windows Server 2025**, with the RemoteFX USB backend
  and the policy set to every user, repeated on this PR's head:
  - A YubiKey 5 was claimed, redirected, and enumerated on the server with
  its composite device, OTP keyboard, FIDO and CCID interfaces all OK.
  - A CTAPHID `INIT` sent to its FIDO interface in the session got its
  nonce echoed back, which exercises interrupt OUT and IN transfers end to
  end.
  - The session stayed up for about an hour, and the YubiKey went back to
  its own drivers when the session ended.
  - Unplugging the YubiKey during the session closed its device channel;
  plugging it back in claimed it again on a new channel, and the CTAPHID
  ping passed on the new device.
  - The server needed the RD Session Host role and `fDisablePNPRedir = 0`
  to offer URBDRC; it isn't offered in remote administration mode.
  - With the nusb backend, URBDRC negotiates; no WinUSB device was
  available to redirect.
  - **Not verified**:
    - a WinUSB device redirected through the nusb backend;
  - smart card access in the session: the reader enumerates, but the
  server's smart card service wasn't reachable;
  - OTP keystrokes reach the server's console rather than the session, as
  with mstsc.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



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
