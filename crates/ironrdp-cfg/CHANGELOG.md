# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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
