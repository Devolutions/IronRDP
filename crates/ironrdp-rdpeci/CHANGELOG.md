# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpeci-v0.0.0)] - 2026-10-31

### <!-- 1 -->Features

- Add MS-RDPECI core input dynamic channel ([#2319](https://github.com/Devolutions/IronRDP/issues/2319)) ([fbf2599863](https://github.com/Devolutions/IronRDP/commit/fbf25998630e1692dbea5479e33820cf0bd70acb)) 

  Add the ironrdp-rdpeci crate with the MS-RDPECI PDUs and a client
  processor for the Microsoft::Windows::RDS::CoreInput dynamic channel.
  Input events reuse the fast-path input event type, since MS-RDPECI packs
  events in the same format.
  
  The client can opt in with the `core-input` feature and
  ChannelConfig::core_input. Once the server opens the channel and selects
  version 1.0, keyboard and mouse input is sent over it; otherwise input
  stays on fast-path.


