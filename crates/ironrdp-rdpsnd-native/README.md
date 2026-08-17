# IronRDP native audio backends

Native audio backend implementations used by IronRDP clients.

## Features

- **Playback (default path)** — CPAL output backend for the RDPSND static channel
  ([MS-RDPEA](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/)).
- **`capture`** — optional CPAL input backend implementing the MS-RDPEAI
  `RdpeaiCaptureHandler` for the `AUDIO_INPUT` dynamic channel.
  Enable with the `capture` feature (pulls in `ironrdp-rdpeai`).

The capture backend lives behind a feature flag so playback-only consumers do not
depend on the AUDIO_INPUT protocol crate. A dedicated `ironrdp-rdpeai-native`
crate may be split out later; until then this crate hosts both native backends.

This crate is part of the [IronRDP] project.

[CPAL]: https://github.com/rustaudio/cpal
[IronRDP]: https://github.com/Devolutions/IronRDP
