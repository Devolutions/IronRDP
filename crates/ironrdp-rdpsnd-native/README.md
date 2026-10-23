# IronRDP native audio backends

Native audio backends used by IronRDP clients, built on [CPAL].

## Features

- **Playback (default path)**: `cpal::RdpsndBackend` implements the RDPSND client handler for the
  static and dynamic audio output channels ([MS-RDPEA]).
  It decodes PCM, G.711, IMA and Microsoft ADPCM, Opus (`opus` feature) and, on Windows, AAC through
  Media Foundation (`aac` feature), and plays through a jitter buffer with a configurable latency
  target. It reports the buffered and device latency for Wave Confirm timing, follows the system
  default device unless a device is selected, and on Windows applies the server volume to the
  application's audio session instead of scaling samples.
- **`capture`**: `RdpeaiCaptureBackend` implements the MS-RDPEAI `RdpeaiCaptureHandler` for the
  `AUDIO_INPUT` dynamic channel. It records 16-bit PCM and encodes it to the negotiated format
  (PCM, G.711 or ADPCM), offering compressed formats first.

`list_devices` enumerates the output and input devices; `DeviceSelector` picks one by ID or name.

The capture backend lives behind a feature flag so playback-only consumers do not
depend on the AUDIO_INPUT protocol crate.

This crate is part of the [IronRDP] project.

[CPAL]: https://github.com/rustaudio/cpal
[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/
