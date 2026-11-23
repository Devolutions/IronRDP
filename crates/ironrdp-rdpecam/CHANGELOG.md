# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpecam-v0.2.0...ironrdp-rdpecam-v0.2.1)] - 2026-11-23

### <!-- 1 -->Features

- Add MS-RDPECAM camera redirection processors ([#2445](https://github.com/Devolutions/IronRDP/issues/2445)) ([b456259eaa](https://github.com/Devolutions/IronRDP/commit/b456259eaa4af861d1105bdfb635c46c88daa773)) 

  Add `EnumerationServer`, `DeviceServer`, and `DesiredCameraFormat` so an
  RDP server can negotiate [MS-RDPECAM] protocol versions, track announced
  client cameras, and drive per-device dynamic virtual channels.
  
  `DeviceServer` activates the device, selects a color stream, negotiates
  an uncompressed or MJPEG format within caller-specified dimension bounds
  (or H.264 when explicitly enabled), issues sample credits as frames are
  delivered, and stops active streams before deactivating the device on
  shutdown or setup/sample failure.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpecam-v0.1.0...ironrdp-rdpecam-v0.2.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11

- [**breaking**] Update `ironrdp-svc` public dependency to 0.10
