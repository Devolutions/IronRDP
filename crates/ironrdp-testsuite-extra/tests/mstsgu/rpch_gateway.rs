// oracle: regression Devolutions/IronRDP#2202.
#[tokio::test]
async fn public_gateway_stream_preserves_data_authentication_and_channel_lifecycle() {
    super::process::run("rpch-gateway", None, None)
        .await
        .expect("isolated public RPCH gateway checks");
}

// oracle: spec MS-TSGU §2.2.9.2 and MS-RPCH §2.1.2.1.4.
#[tokio::test]
async fn public_gateway_stream_rejects_malformed_authenticated_replies() {
    super::process::run("rpch-malformed", None, None)
        .await
        .expect("isolated malformed gateway reply checks");
}

// oracle: spec MS-RPCH §2.1.2.1.1 and §3.2.2.5.5.
#[tokio::test]
#[ignore = "transfers more than 1 GiB through the public gateway stream"]
async fn public_gateway_stream_preserves_data_across_in_channel_rollover() {
    super::process::run("rpch-in-recycle", None, None)
        .await
        .expect("isolated full-size IN rollover check");
}
