mod autodetect;
mod connection_activation;
mod fast_path;

// Pulled in from the crate itself: `ironrdp-session` sets `test = false`, so its
// inline unit tests only run when compiled as part of this test suite.
#[path = "../../../ironrdp-session/src/qoiz.rs"]
mod qoiz;
mod rfx;
mod save_session_info;

#[cfg(test)]
mod tests {
    use ironrdp_pdu::rdp::capability_sets::{CodecProperty, client_codecs_capabilities};

    #[test]
    fn test_codecs_capabilities() {
        let config = &[];
        let _capabilities = client_codecs_capabilities(config).unwrap();

        let config = &["badcodec"];
        assert!(client_codecs_capabilities(config).is_err());

        let config = &["remotefx:on"];
        let capabilities = client_codecs_capabilities(config).unwrap();
        assert!(
            capabilities
                .0
                .iter()
                .any(|cap| matches!(cap.property, CodecProperty::RemoteFx(_)))
        );

        let config = &["remotefx:off"];
        let capabilities = client_codecs_capabilities(config).unwrap();
        assert!(
            !capabilities
                .0
                .iter()
                .any(|cap| matches!(cap.property, CodecProperty::RemoteFx(_)))
        );

        let config = &["qoi:on"];
        let capabilities = client_codecs_capabilities(config).unwrap();
        assert!(
            capabilities
                .0
                .iter()
                .any(|cap| matches!(cap.property, CodecProperty::Qoi))
        );
    }
}
