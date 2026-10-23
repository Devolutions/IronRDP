macro_rules! check {
    ($oracle:ident) => {{
        use ironrdp_fuzzing::oracles;

        const REGRESSION_DATA_FOLDER: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            concat!("/test_data/fuzz_regression/", stringify!($oracle))
        );

        println!("Read directory {REGRESSION_DATA_FOLDER}");
        for entry in std::fs::read_dir(REGRESSION_DATA_FOLDER).unwrap() {
            let entry = entry.unwrap();
            println!("Check {}", entry.path().display());
            let test_case = std::fs::read(entry.path()).unwrap();
            oracles::$oracle(&test_case);
        }
    }};
}

#[test]
fn check_pdu_decode() {
    check!(pdu_decode);
}

#[test]
fn check_cliprdr_format() {
    check!(cliprdr_format);
}

#[test]
fn check_bulk_decompress_mppc() {
    check!(bulk_decompress_mppc);
}

#[test]
fn check_bulk_decompress_ncrush() {
    check!(bulk_decompress_ncrush);
}

#[test]
fn check_bulk_decompress_xcrush() {
    check!(bulk_decompress_xcrush);
}

#[test]
fn check_bulk_decompress_rdp8() {
    check!(bulk_decompress_rdp8);
}

#[test]
fn check_bulk_round_trip() {
    check!(bulk_round_trip);
}

#[test]
fn check_pdu_round_trip() {
    check!(pdu_round_trip);
}

#[test]
fn check_channel_process() {
    check!(channel_process);
}

#[test]
fn check_egfx_round_trip() {
    check!(egfx_round_trip);
}

#[test]
fn check_egfx_avc420_decode() {
    check!(egfx_avc420_decode);
}

#[test]
fn check_egfx_multi_frame() {
    check!(egfx_multi_frame);
}

#[test]
fn check_rdpeusb_decode() {
    check!(rdpeusb_decode);
}

#[test]
fn check_message_decoding_invariants() {
    check!(message_decoding_invariants);
}

#[test]
fn check_egfx_avc444_decode() {
    check!(egfx_avc444_decode);
}

#[test]
fn check_egfx_zgfx_decompress() {
    check!(egfx_zgfx_decompress);
}

#[test]
fn check_egfx_surface_state() {
    check!(egfx_surface_state);
}

// oracle: spec crates/ironrdp-fuzzing/src/oracles/mod.rs §rdpsnd_codec_decode (no panic for any accepted format; seeds built from the FFmpeg vectors in crates/ironrdp-testsuite-core/test_data/audio)
#[test]
fn check_rdpsnd_codec_decode() {
    check!(rdpsnd_codec_decode);
}
