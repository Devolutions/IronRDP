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
pub fn check_pdu_decode() {
    check!(pdu_decode);
}

#[test]
pub fn check_cliprdr_format() {
    check!(cliprdr_format);
}
