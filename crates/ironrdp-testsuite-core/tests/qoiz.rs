use ironrdp_server::QoizCompressor;
use ironrdp_session::SessionResult;
use ironrdp_session::{ZripDecompressor, ZstdSafeDecompressor};

// Four updates of a single zstd stream, captured from the `zstd-safe` backend
// (the reference C implementation) for `payloads()`.
const ZSTD_SAFE_STREAM: &[&[u8]] = &[
    &[
        0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x88, 0x44, 0x04, 0x00, 0x12, 0x88, 0x1d, 0x07, 0x10, 0x10, 0xdf, 0x30, 0xb3,
        0x7f, 0x0a, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x75, 0xfa, 0x5c, 0x1e, 0x87, 0xbf, 0xdd, 0x6d, 0xf6, 0x5a,
        0x9d, 0x46, 0x9f, 0xcd, 0x65, 0xf2, 0x58, 0x1c, 0x06, 0x7f, 0xbd, 0x5d, 0xee, 0x56, 0x9b, 0xc5, 0x5e, 0xad,
        0x55, 0xea, 0x54, 0x1a, 0x85, 0x3e, 0x9d, 0x4d, 0xe6, 0x52, 0x99, 0x44, 0x1e, 0x8d, 0x45, 0xe2, 0x50, 0x18,
        0x04, 0xfe, 0x7c, 0x3d, 0xde, 0x4e, 0x97, 0xc3, 0xdd, 0x6c, 0x35, 0xda, 0x4c, 0x16, 0x83, 0xbd, 0x5c, 0x2d,
        0xd6, 0x4a, 0x95, 0x42, 0x9d, 0x4c, 0x25, 0xd2, 0x48, 0x14, 0x02, 0x7d, 0x3c, 0x1d, 0xce, 0x46, 0x93, 0xc1,
        0x5c, 0x2c, 0x15, 0xca, 0x44, 0x12, 0x81, 0x3c, 0x1c, 0x0d, 0xc6, 0x42, 0x91, 0x40, 0x1c, 0x0c, 0x05, 0xc2,
        0x40, 0x10, 0xd8, 0xf7, 0x78, 0x98, 0x10, 0xf0, 0x07, 0x00, 0x10, 0x7e, 0xc5, 0x0f, 0xd5, 0xda, 0x38, 0x48,
        0x01,
    ],
    &[
        0x94, 0x00, 0x00, 0x40, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x02, 0x00, 0xb8, 0x02, 0xd8, 0x97,
        0xff, 0xcf, 0x40,
    ],
    &[0x44, 0x00, 0x00, 0x00, 0x01, 0x00, 0xfd, 0x0f, 0xc0, 0x7f, 0x80],
    &[
        0x94, 0x00, 0x00, 0x40, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x02, 0x00, 0xb8, 0x02, 0xd8, 0x97,
        0xff, 0x0f, 0x81,
    ],
];

// The same updates compressed by a third-party encoder (zrip).
// It uses a smaller window and enables the content checksum, which the decompressor must also accept.
const THIRD_PARTY_STREAM: &[&[u8]] = &[
    &[
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x48, 0x44, 0x04, 0x00, 0x04, 0x08, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
        0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
        0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a,
        0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
        0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e,
        0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60,
        0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72,
        0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x54, 0x01, 0x00, 0x0d,
        0x01,
    ],
    &[
        0x9c, 0x00, 0x00, 0x48, 0x78, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x02, 0x00, 0x2e, 0x21, 0xec,
        0xcb, 0xff, 0x67, 0x20,
    ],
    &[0x44, 0x00, 0x00, 0x00, 0x01, 0x00, 0xfd, 0x0f, 0xc0, 0x7f, 0x80],
    &[
        0x9c, 0x00, 0x00, 0x48, 0x79, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x02, 0x00, 0x2f, 0x21, 0xec,
        0xcb, 0xff, 0x87, 0x40,
    ],
];

/// Magic bytes every zstd frame starts with.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

type CompressFn = Box<dyn FnMut(&[u8]) -> Vec<u8>>;
type DecompressFn = Box<dyn FnMut(&[u8]) -> SessionResult<Vec<u8>>>;

/// Payload of the update at index `i` in the fixed streams.
fn payload(i: u8) -> Vec<u8> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "`j / 17` stays under 121, so the conversion never truncates"
    )]
    (0..2048u32).map(|j| ((j / 17) as u8).wrapping_add(i)).collect()
}

/// The updates encoded by the fixed streams.
/// The third repeats the first, so decoding it requires the history of the earlier updates.
fn payloads() -> Vec<Vec<u8>> {
    vec![payload(0), payload(1), payload(0), payload(2)]
}

/// Deterministic, incompressible data.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;

    core::iter::repeat_with(|| {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state.to_le_bytes()[0]
    })
    .take(len)
    .collect()
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    Zrip,
    Zstd,
}

/// A fresh compressor.
fn compressor() -> CompressFn {
    let mut zstd = QoizCompressor::new().unwrap();
    Box::new(move |input| zstd.compress(input).unwrap())
}

impl Backend {
    /// A fresh decompressor of this backend.
    fn decompressor(self) -> DecompressFn {
        match self {
            Self::Zrip => {
                let mut zrip = ZripDecompressor::default();
                Box::new(move |input| zrip.decompress(input))
            }
            Self::Zstd => {
                let mut zstd = ZstdSafeDecompressor::default();
                Box::new(move |input| zstd.decompress(input))
            }
        }
    }
}

// oracle: property round-trip
#[rstest::rstest]
fn compressor_decompressor_round_trip_matrix(#[values(Backend::Zrip, Backend::Zstd)] dec: Backend) {
    let mut compress = compressor();
    let mut decompress = dec.decompressor();

    for (i, expected) in payloads().into_iter().enumerate() {
        assert_eq!(
            decompress(&compress(&expected)).unwrap(),
            expected,
            "{dec:?}: update {i} differs"
        );
    }
}

// oracle: spec RFC 8878 §3.1.1 (a frame starts with the magic number 0xFD2FB528, stored little-endian)
#[rstest::rstest]
fn produces_a_zstd_frame() {
    let compressed = compressor()(&payload(0));

    assert_eq!(compressed.get(..4), Some(ZSTD_MAGIC.as_slice()), "zstd frame");
}

// oracle: mutant compressing each update without the history of the earlier ones
#[rstest::rstest]
fn reuses_history_across_updates() {
    let without_history = compressor()(&payload(0)).len();

    let mut compress = compressor();
    compress(&payload(0));
    compress(&payload(1));
    let with_history = compress(&payload(0)).len();

    assert!(
        with_history < without_history,
        "history did not shrink the repeated update: {with_history} bytes with, {without_history} bytes without"
    );
}

// oracle: reference zstd-safe capture and zrip capture of `payloads()` (both decoders accept frames from either encoder)
#[rstest::rstest]
fn decompresses_fixed_stream(
    #[values(ZSTD_SAFE_STREAM, THIRD_PARTY_STREAM)] stream: &[&[u8]],
    #[values(Backend::Zrip, Backend::Zstd)] dec: Backend,
) {
    let mut decompress = dec.decompressor();

    for (i, (update, expected)) in stream.iter().zip(payloads()).enumerate() {
        assert_eq!(decompress(update).unwrap(), expected, "{dec:?}: update {i} differs");
    }
}

/// Updates whose last one repeats the first from 4 MiB back, beyond the default 2 MiB window at compression level 3.
fn distant_updates() -> Vec<Vec<u8>> {
    const UPDATE_LEN: usize = 1 << 20;

    let first = noise(1, UPDATE_LEN);
    let mut updates = vec![first.clone()];
    updates.extend((2..5).map(|seed| noise(seed, UPDATE_LEN)));
    updates.push(first);
    updates
}

// oracle: mutant compressing each update without reaching history beyond the default window
#[rstest::rstest]
fn matches_against_distant_updates() {
    let updates = distant_updates();

    let mut compress = compressor();
    let mut with_history = 0;

    for update in &updates {
        with_history = compress(update).len();
    }

    let without_history = compressor()(&updates[0]).len();

    // Only long distance matching reaches the repeat.
    assert!(
        with_history * 100 < without_history,
        "repeated update did not match against distant history: {with_history} bytes with, {without_history} bytes without"
    );
}

// oracle: mutant decoding a back-reference beyond the default window incorrectly or rejecting the long-window frame
#[rstest::rstest]
fn decompresses_distant_history(#[values(Backend::Zrip, Backend::Zstd)] dec: Backend) {
    let updates = distant_updates();

    let mut compress = compressor();
    let mut decompress = dec.decompressor();

    for (i, expected) in updates.iter().enumerate() {
        let compressed = compress(expected);
        assert_eq!(
            decompress(&compressed).unwrap(),
            *expected,
            "{dec:?}: update {i} differs"
        );
    }
}

// oracle: property round-trip
#[rstest::rstest]
fn round_trips_large_updates(#[values(Backend::Zrip, Backend::Zstd)] dec: Backend) {
    // The repetitive update expands far beyond the initial output estimate.
    // The noise compresses to more than its own size.
    let updates = [("repetitive", vec![0x42; 1 << 20]), ("noise", noise(1, 300 * 1024))];

    let mut compress = compressor();
    let mut decompress = dec.decompressor();

    for (name, update) in &updates {
        assert_eq!(
            decompress(&compress(update)).unwrap(),
            *update,
            "{dec:?}: {name} differs"
        );
    }
}

// oracle: mutant failing on an empty update or losing the stream history after one
#[rstest::rstest]
fn decompresses_empty_update(#[values(Backend::Zrip, Backend::Zstd)] dec: Backend) {
    let mut decompress = dec.decompressor();

    assert_eq!(decompress(&[]).unwrap(), Vec::<u8>::new(), "{dec:?}");

    for (update, expected) in ZSTD_SAFE_STREAM.iter().zip(payloads()) {
        assert_eq!(decompress(update).unwrap(), expected, "{dec:?}");
        assert_eq!(decompress(&[]).unwrap(), Vec::<u8>::new(), "{dec:?}");
    }
}

// oracle: mutant decoding an update whose predecessors were skipped instead of rejecting it
// An update is only decodable after the ones before it, so joining mid-stream must fail rather than produce garbage.
#[rstest::rstest]
fn rejects_stream_joined_late(#[values(Backend::Zrip, Backend::Zstd)] dec: Backend) {
    let result = dec.decompressor()(ZSTD_SAFE_STREAM[1]);

    assert!(result.is_err(), "{dec:?}: expected an error, got {result:?}");
}
