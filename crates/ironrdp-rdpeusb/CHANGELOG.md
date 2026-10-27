# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpeusb-v0.1.0...ironrdp-rdpeusb-v0.2.0)] - 2026-10-27

### <!-- 1 -->Features

- [**breaking**] Add `DeviceInfo::isochronous`; `ADD_DEVICE` advertises a no-ack isochronous jitter buffer only when it is set ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

- Add `encode_ts_urb` and `decode_ts_urb_result` to `TsUrbInPacket` and `TsUrbOutPacket`, for backends that hand `TS_URB`s to the operating system ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

- `CANCEL_REQUEST` keeps the request pending until the backend completes it ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

### <!-- 4 -->Bug Fixes

- Accept `REGISTER_REQUEST_CALLBACK` interface IDs below 4, which Windows sends; the session ended right after a device was announced ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

- Accept control requests without a data stage in `TRANSFER_IN_REQUEST` messages that keep the OUT direction, as Windows sends them ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

- A refused device channel no longer uses up an interface ID ([#2181](https://github.com/Devolutions/IronRDP/issues/2181)) ([35247071a3](https://github.com/Devolutions/IronRDP/commit/35247071a32a718094fc4df1de60e220e3730f4b))

- Decoding truncated input returns an error instead of panicking ([#2130](https://github.com/Devolutions/IronRDP/issues/2130)) ([b69cda22ec](https://github.com/Devolutions/IronRDP/commit/b69cda22ec692e212caffdbb9fe0c4e5ff1d2696))

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11
