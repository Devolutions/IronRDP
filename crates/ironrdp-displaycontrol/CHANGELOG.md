# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.8.0...ironrdp-displaycontrol-v0.9.0)] - 2026-10-23

### <!-- 1 -->Features

- Add `DisplayControlHandler::capabilities`, defaulting to the new `DisplayControlCapabilities::single_monitor`, so a server handler can advertise more monitors ([#1917](https://github.com/Devolutions/IronRDP/issues/1917)) ([db0066072e](https://github.com/Devolutions/IronRDP/commit/db0066072e787ea1268acf2ce46ed5ff0bb578b9)) ([#1918](https://github.com/Devolutions/IronRDP/issues/1918)) ([25331598f3](https://github.com/Devolutions/IronRDP/commit/25331598f37f49573205c54ee45c878d291317f2))

- Add multi-monitor layouts on the client: `DisplayControlClient::encode_monitor_layout` sends a full layout fitted with the new `DisplayControlMonitorLayout::fit_to_capabilities`, which drops monitors beyond the server's `MaxNumMonitors`, scales down an oversized single monitor and rejects an oversized multi-monitor layout ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

- Add `DisplayControlClient::capabilities`, `DisplayControlCapabilities::max_num_monitors`, `DisplayControlMonitorLayout::desktop_size`, `MonitorLayoutEntry::adjust_desktop_scale_factor`, `DeviceScaleFactor::from_value` and `DeviceScaleFactor::from_desktop_scale_factor` ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

- Fit the layout sent by `DisplayControlClient::encode_single_primary_monitor` to the server capabilities ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

- Derive the device scale factor from the desktop scale factor, clamped to 100–500%, in `DisplayControlMonitorLayout::new_single_primary_monitor` instead of always using 100% ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

- [**breaking**] Reject overlapping monitors, and in multi-monitor layouts monitors not adjacent to another one, in `DisplayControlMonitorLayout::new` ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

### <!-- 4 -->Bug Fixes

- Decode the headered `DISPLAYCONTROL_CAPS_PDU` in `DisplayControlClient`, and reject display control PDUs whose length does not match their payload ([#1442](https://github.com/Devolutions/IronRDP/issues/1442)) ([3b66961a8b](https://github.com/Devolutions/IronRDP/commit/3b66961a8b2ec5bb2d49175c6970e4a480348b3f))

- Use landscape orientation for a square monitor in `DisplayControlMonitorLayout::new_single_primary_monitor` ([#2182](https://github.com/Devolutions/IronRDP/issues/2182)) ([0784fceca8](https://github.com/Devolutions/IronRDP/commit/0784fceca8d1c29cdceda2c74339e13311f0b857))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.9

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.10

- [**breaking**] Update `ironrdp-svc` public dependency to 0.9



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.7.0...ironrdp-displaycontrol-v0.8.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.8

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.6.0...ironrdp-displaycontrol-v0.7.0)] - 2026-06-05

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.5.0...ironrdp-displaycontrol-v0.6.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-core`, `ironrdp-dvc`, `ironrdp-pdu`, and `ironrdp-svc` public dependencies

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.3...ironrdp-displaycontrol-v0.2.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.2...ironrdp-displaycontrol-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.1...ironrdp-displaycontrol-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-displaycontrol-v0.1.0...ironrdp-displaycontrol-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 
