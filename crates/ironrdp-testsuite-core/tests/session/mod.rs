mod rfx;

#[cfg(test)]
mod tests {
    use ironrdp_pdu::rdp::capability_sets::{client_codecs_capabilities, CodecProperty};

    #[test]
    fn test_codecs_capabilities() {
        let config = &[];
        let _capabilities = client_codecs_capabilities(config).unwrap();

        let config = &["badcodec"];
        assert!(client_codecs_capabilities(config).is_err());

        let config = &["remotefx:on"];
        let capabilities = client_codecs_capabilities(config).unwrap();
        assert_eq!(capabilities.0.len(), 1);
        assert!(matches!(capabilities.0[0].property, CodecProperty::RemoteFx(_)));

        let config = &["remotefx:off"];
        let capabilities = client_codecs_capabilities(config).unwrap();
        assert_eq!(capabilities.0.len(), 0);
    }
}
