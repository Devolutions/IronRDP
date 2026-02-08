// NCRUSH (RDP 6.0) test data.
//
// Test vectors ported from FreeRDP TestFreeRDPCodecNCrush.c
//
// Original copyright:
//   Copyright 2014 Marc-Andre Moreau <marcandre.moreau@gmail.com>
//   Licensed under the Apache License, Version 2.0

/// Plaintext "bells" test string used by both compress and decompress tests.
pub(super) const TEST_BELLS_DATA: &[u8] = b"for.whom.the.bell.tolls,.the.bell.tolls.for.thee!";

/// NCRUSH-compressed form of `TEST_BELLS_DATA`.
///
/// Produced by FreeRDP's `ncrush_compress` with a freshly-created compressor
/// context (`ncrush_context_new(TRUE)`).
#[rustfmt::skip]
pub(super) const TEST_BELLS_NCRUSH: &[u8] = &[
    0xfb, 0x1d, 0x7e, 0xe4, 0xda, 0xc7, 0x1d, 0x70,
    0xf8, 0xa1, 0x6b, 0x1f, 0x7d, 0xc0, 0xbe, 0x6b,
    0xef, 0xb5, 0xef, 0x21, 0x87, 0xd0, 0xc5, 0xe1,
    0x85, 0x71, 0xd4, 0x10, 0x16, 0xe7, 0xda, 0xfb,
    0x1d, 0x7e, 0xe4, 0xda, 0x47, 0x1f, 0xb0, 0xef,
    0xbe, 0xbd, 0xff, 0x2f,
];
