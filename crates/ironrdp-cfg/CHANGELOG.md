# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.2.0...ironrdp-cfg-v0.3.0)] - 2026-10-24

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



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cfg-v0.1.0...ironrdp-cfg-v0.2.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Add the required `PropertySetExt` accessors for `ironrdp_named_pipe` and `ironrdp_sandbox_id` (`named_pipe`, `sandbox_id`) ([#1580](https://github.com/Devolutions/IronRDP/issues/1580)) ([39b020343d](https://github.com/Devolutions/IronRDP/commit/39b020343d962962bfbefc89939be64d5c716196))

- [**breaking**] Add the required `PropertySetExt` accessors for `remoteapplicationmode` (`remote_application_mode`) ([#1637](https://github.com/Devolutions/IronRDP/issues/1637)) ([ab48c6cb8c](https://github.com/Devolutions/IronRDP/commit/ab48c6cb8c017504f8a92799aeb91b821c50a13a))

- [**breaking**] Add the required `PropertySetExt` accessors for `audiocapturemode` (`audio_capture_mode`), typed as the new `AudioCaptureMode` with `UnknownAudioCaptureMode` for unrecognized values ([#1642](https://github.com/Devolutions/IronRDP/issues/1642)) ([205fe038cc](https://github.com/Devolutions/IronRDP/commit/205fe038cc693598adf803fe181526b789b2ec3d))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectwebauthn` (`redirect_webauthn`) ([#1644](https://github.com/Devolutions/IronRDP/issues/1644)) ([66da78bc4e](https://github.com/Devolutions/IronRDP/commit/66da78bc4e6b37a7780dbf9f333234be63d96afb))

- [**breaking**] Add the required `PropertySetExt` accessors for `ironrdp_vmconnect`, `ironrdp_vmconnect_basic` and `ironrdp_vmconnect_current_user` (`vmconnect_id`, `vmconnect_basic`, `vmconnect_current_user`) ([#1768](https://github.com/Devolutions/IronRDP/issues/1768)) ([cf9bdef764](https://github.com/Devolutions/IronRDP/commit/cf9bdef764da6b9aff33b436662441e1503dc93a))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectprinters` (`redirect_printers`) ([#2156](https://github.com/Devolutions/IronRDP/issues/2156)) ([408b86ea91](https://github.com/Devolutions/IronRDP/commit/408b86ea91dc6b953778278ee2ffbd99756688f5))

- [**breaking**] Add the required `PropertySetExt` accessors for `redirectcomports` (`redirect_com_ports`) ([#2187](https://github.com/Devolutions/IronRDP/issues/2187)) ([850839f475](https://github.com/Devolutions/IronRDP/commit/850839f4756f4efb486a47abb89a93b3b4694a40))

- [**breaking**] Add the required `PropertySetExt` accessors for `drivestoredirect` and `redirectdrives` (`drives_to_redirect`, `redirect_drives`), plus `drive_redirection` and `set_drive_redirection`, which resolve both keys into the new `DriveRedirection` (mstsc `drivestoredirect` syntax: `*`, `DynamicDrives`, drive letters) ([#2178](https://github.com/Devolutions/IronRDP/issues/2178)) ([1de38a2ee5](https://github.com/Devolutions/IronRDP/commit/1de38a2ee55346a577ec28f3706c81dd3f7e8316))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-cfg-v0.1.0)] - 2026-07-10

Initial release.
