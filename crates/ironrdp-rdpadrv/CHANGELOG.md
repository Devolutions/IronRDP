# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpadrv-v0.0.0)] - 2026-10-30

### <!-- 1 -->Features

- Implement the MS-RDPADRV audio level (`WMSAud`) and drive letter persistence (`WMSDL`) dynamic virtual channels ([#2322](https://github.com/Devolutions/IronRDP/issues/2322)) ([8301ee04d3](https://github.com/Devolutions/IronRDP/commit/8301ee04d32f580f9d7582267a063de1812e1afb))

  `AudioLevelPdu` and `DriveLetterPdu` encode and decode both channels' messages.
  `AudioLevelClient` and `DriveLetterClient` store what the server reports and send the stored values back when the server starts the protocol.
  `AudioLevelServer` and `DriveLetterServer` start the protocol and hand the values the client restores to a handler.
