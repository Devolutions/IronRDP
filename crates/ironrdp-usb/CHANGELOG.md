# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-usb-v0.1.0)] - 2026-09-11

### <!-- 1 -->Features

- Add the ironrdp-usb crate ([#1682](https://github.com/Devolutions/IronRDP/issues/1682)) ([cec0c1487f](https://github.com/Devolutions/IronRDP/commit/cec0c1487fad817435a9a70de387d2c57eb9caad)) 

  Introduce a protocol-independent, `no_std`, sans-I/O crate holding
  USB-standard data structures and semantics that are shared by any USB
  redirection transport adapter:
  
  - typed USB values and identifiers;
  - descriptor storage, parsing, validation, and traversal for device,
  configuration, interface, and endpoint descriptors (`descriptor`);
  - standard control requests and setup packets (`control`);
  - endpoint identity, addressing, and companion metadata (`endpoint`);
  - transfer direction, type, status, and request/result data
  (`transfer`).
  
  The crate is dependency-free, borrows its parsing input, and is generic
  over caller-owned buffer storage, so it never allocates. It describes
  USB operations and data but does not execute them: device I/O, async
  runtime integration, request routing, and pending-request management all
  stay in the consuming protocol layers.
  
  **Parsing is restricted to byte layouts defined by USB itself**. RDPEUSB
  PDUs, usbredir packets, and other transport framing belong to their
  respective crates.
  
  Descriptor parsing is a hostile-input surface, so the tests in
  `ironrdp-testsuite-core` cover the framing, length, and topology
  boundaries rather than the internals: `bLength` framing, minimum and
  trailing-byte rules, `wTotalLength` completeness, the configuration
  topology rules enforced by `validate`, setup packet size and field
  partitioning, and the endpoint address and packet-size encodings.

- [**breaking**] Support SuperSpeed packet sizes ([#1883](https://github.com/Devolutions/IronRDP/issues/1883)) ([8fd0b174a9](https://github.com/Devolutions/IronRDP/commit/8fd0b174a969d8166053d5cdea59b923e4dc4cd5)) 

  `ironrdp-usb` could not describe a SuperSpeed device:
  `is_valid_for_usb2` rejected every SuperSpeed endpoint, and
  `endpoint_zero_max_packet_size` required a bus speed to read
  `bMaxPacketSize0`.
  
  That speed parameter added nothing. USB 3.2 Section 9.6.1 admits only 9
  there, as a base-2 exponent, while USB 2.0 admits only 8, 16, 32 and 64,
  stated literally, so the two encodings are disjoint and the value
  identifies itself. Requiring a speed only let a caller contradict the
  descriptor, which is exactly what a transport that cannot express
  SuperSpeed ends up doing.
  
  Endpoint zero now decodes from the value alone, and the endpoint
  predicate carries the USB 3.2 Section 9.6.6 limits, so
  `is_valid_for_usb2` and `usb2_max_payload` lose their USB 2.0 names.


