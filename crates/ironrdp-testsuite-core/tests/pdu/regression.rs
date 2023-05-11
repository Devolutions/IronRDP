const REGRESSION_DATA_FOLDER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test_data/pdu/regression/");

#[test]
pub fn check() {
    for entry in std::fs::read_dir(REGRESSION_DATA_FOLDER).unwrap() {
        let entry = entry.unwrap();
        println!("Check {}", entry.path().display());
        let test_case = std::fs::read(entry.path()).unwrap();
        ironrdp_fuzzing::oracles::pdu_decode(&test_case);
    }
}
