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
fn check_bulk_round_trip() {
    check!(bulk_round_trip);
}

#[test]
fn check_pdu_round_trip() {
    check!(pdu_round_trip);
}

#[test]
fn check_egfx_round_trip() {
    check!(egfx_round_trip);
}

#[test]
fn check_message_decoding_invariants() {
    check!(message_decoding_invariants);
}
