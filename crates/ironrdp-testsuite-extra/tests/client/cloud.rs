use ironrdp_client::cloud::{
    AvdCloudEnvironment, AvdResourceKind, CloudPcClient, Error, avd_launch_url, parse_avd_feed_discovery_xml,
    parse_avd_workspace_feed_xml, windows365_launch_url,
};

const PC_ID: &str = "662009bc-7732-4f6f-8726-25883518ffff";
const WORKSPACE_ID: &str = "00000000-0000-0000-0000-000000000000";
const TENANT_ID: &str = "11111111-1111-1111-1111-111111111111";

#[test]
fn documented_launch_urls() {
    assert_eq!(
        windows365_launch_url(PC_ID, None).unwrap(),
        format!("https://windows.cloud.microsoft/webclient/ent/{PC_ID}")
    );
    assert_eq!(
        avd_launch_url(WORKSPACE_ID, PC_ID, Some(TENANT_ID)).unwrap(),
        format!("https://windows.cloud.microsoft/webclient/avd/{WORKSPACE_ID}/{PC_ID}?tenant={TENANT_ID}")
    );
    assert_eq!(
        windows365_launch_url(PC_ID, Some(TENANT_ID)).unwrap(),
        format!("https://windows.cloud.microsoft/webclient/ent/{PC_ID}?tenant={TENANT_ID}")
    );
    assert_eq!(
        avd_launch_url(WORKSPACE_ID, PC_ID, None).unwrap(),
        format!("https://windows.cloud.microsoft/webclient/avd/{WORKSPACE_ID}/{PC_ID}")
    );
}

#[test]
fn launch_urls_reject_missing_or_injected_ids() {
    for invalid in [
        "",
        ".",
        "..",
        "/path",
        "id?token=secret",
        "id#fragment",
        "a b",
        "a\\b",
        "\r\n",
    ] {
        assert!(matches!(
            windows365_launch_url(invalid, None),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            avd_launch_url(invalid, PC_ID, None),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            avd_launch_url(WORKSPACE_ID, invalid, None),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            windows365_launch_url(PC_ID, Some(invalid)),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[tokio::test]
async fn invalid_tokens_and_continuations_fail_before_network() {
    let client = CloudPcClient::new().unwrap();
    for token in ["", "not a token", "secret\r\nInjected: header"] {
        assert!(matches!(
            client.list_page(token, None).await,
            Err(Error::InvalidInput("Graph bearer token"))
        ));
    }
    for next in [
        "http://127.0.0.1/v1.0/me/cloudPCs",
        "https://attacker.invalid/v1.0/me/cloudPCs",
        "https://graph.microsoft.com/v1.0/users",
    ] {
        assert!(matches!(
            client.list_page("token", Some(next)).await,
            Err(Error::InvalidContinuation)
        ));
    }
    assert!(matches!(
        client.retrieve_launch_detail("token", "id/path").await,
        Err(Error::InvalidInput("Cloud PC ID"))
    ));
    assert!(matches!(
        client.retrieve_launch_detail("not a token", PC_ID).await,
        Err(Error::InvalidInput("Graph bearer token"))
    ));
}

#[test]
fn avd_feed_discovery_and_workspace_xml_parsing() {
    let url = AvdCloudEnvironment::Public.feed_discovery_url(Some(TENANT_ID)).unwrap();
    assert_eq!(
        url.as_str(),
        format!("https://www.wvd.microsoft.com/api/arm/feeddiscovery?aadtenant={TENANT_ID}")
    );

    let feed_discovery_xml = format!(
        r#"<FeedDiscovery><Publisher TenantId="{WORKSPACE_ID}" TenantDisplayName="TestWS" FeedURL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/{WORKSPACE_ID}/webfeed?geo=CA" /></FeedDiscovery>"#
    );
    let subs = parse_avd_feed_discovery_xml(&feed_discovery_xml).unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].tenant_id, WORKSPACE_ID);

    let ws_xml = format!(
        r#"<ResourceCollection><Publisher ID="{WORKSPACE_ID}" Name="TestWS"><Resources><Resource ID="{PC_ID}" Title="SessionDesktop" Type="Desktop"><HostingTerminalServers><HostingTerminalServer><ResourceFile FileExtension=".rdp" URL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/{WORKSPACE_ID}/rdps/{PC_ID}.rdp" ETag="v1" /></HostingTerminalServer></HostingTerminalServers></Resource></Resources></Publisher></ResourceCollection>"#
    );
    let ws = parse_avd_workspace_feed_xml(&ws_xml).unwrap();
    assert_eq!(ws.resources.len(), 1);
    assert_eq!(ws.resources[0].kind, AvdResourceKind::Desktop);
    assert_eq!(ws.resources[0].id, PC_ID);
}
