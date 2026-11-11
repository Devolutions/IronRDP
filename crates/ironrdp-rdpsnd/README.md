# IronRDP RDPSND

Audio output channels implemented as described in [MS-RDPEA]: the static `RDPSND` channel, and the
`AUDIO_PLAYBACK_DVC` and `AUDIO_PLAYBACK_LOSSY_DVC` dynamic channels that Windows 8 and later
servers prefer and route over the UDP transport when one is up.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b

The client processors share one protocol core and drive an `RdpsndClientHandler`, which picks the
formats to advertise from the server's list and plays the wave data:

- `Rdpsnd` serves the static channel, including the pre-v8 WaveInfo / bare Wave two-message path and
  the legacy UDP datagrams (`SNDUDPWAVE` / `SNDUDPWAVELAST`, signature-checked) for a caller-owned
  socket. Wave Encrypt PDUs are not decrypted.
- `RdpsndDvcState` serves both dynamic channels: Windows negotiates on `AUDIO_PLAYBACK_DVC` and sends
  AAC wave data on `AUDIO_PLAYBACK_LOSSY_DVC`, so the two channels share the negotiated state.
- Wave Confirm PDUs report the server timestamp plus the time the client held the block plus the
  handler's playback latency, measured with a caller-supplied clock.

The crate performs no I/O. It also provides:

- `codec`: decoders and encoders for PCM, G.711 A-law and µ-law, IMA ADPCM and Microsoft ADPCM;
- `playout`: a jitter-buffer policy that keeps a latency target, rebuffers after underruns, absorbs
  clock drift and discards backlogs.
