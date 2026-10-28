use ironrdp_mstsgu::GwTunnelPolicy;

// oracle: spec MS-TSGU §2.2.10.17.
#[rstest::rstest]
#[case::absent(
    &[0; 8],
    GwTunnelPolicy { redirection_flags: None, idle_timeout_minutes: None, soh_response: None }
)]
#[case::all_fields(
    &[
        0, 0, 0, 0, // errorCode
        7, 0, // fieldsPresent
        0, 0, // reserved
        8, 0, 0, 0x40, // redirFlags
        15, 0, 0, 0, // idleTimeout
        3, 0, 1, 2, 3, // SoHResponse
    ],
    GwTunnelPolicy { redirection_flags: Some(0x4000_0008), idle_timeout_minutes: Some(15), soh_response: Some(vec![1, 2, 3]) }
)]
#[tokio::test]
async fn gateway_reports_authorization_policy(#[case] response: &[u8], #[case] expected: GwTunnelPolicy) {
    let policy = super::consent::connect_gateway_policy(response).await;
    assert_eq!(policy, expected);
}
