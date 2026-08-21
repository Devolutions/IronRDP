mod autodetect;

use ironrdp_connector::{SequenceError, SequenceErrorExt as _};

#[test]
fn connector_error_display_preserves_sequence_error_detail() {
    let sequence_error = SequenceError::reason("Capabilities Exchange", "server rejected the requested color depth");
    let connector_error = ironrdp_connector::map_sequence_error(sequence_error);

    assert_eq!(
        connector_error.to_string(),
        "[sequence error] [Capabilities Exchange] reason: server rejected the requested color depth"
    );
}
