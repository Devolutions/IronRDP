use core::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ironrdp_client::cloud::{
    AvdCloudEnvironment, AvdResourceKind, CloudPcClient, CloudPcPage, Error, MAX_SIGN_SCOPE_CHARS,
    MSTSC_SENSITIVE_SETTINGS, avd_launch_url, continuation_url, launch_detail_request_url,
    parse_avd_feed_discovery_xml, parse_avd_workspace_feed_xml, verify_rdp_signature_at, verify_signed_attrs_for_test,
    windows365_launch_url,
};
use proptest::prelude::*;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

const CLOUD_PCS_URL: &str = "https://graph.microsoft.com/v1.0/me/cloudPCs";
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const PC_ID: &str = "662009bc-7732-4f6f-8726-25883518ffff";
const WORKSPACE_ID: &str = "00000000-0000-0000-0000-000000000000";
const TENANT_ID: &str = "11111111-1111-1111-1111-111111111111";

const AVD_RDP_SIGNATURE_BIN: &[u8] = include_bytes!("avd_rdp_signature.bin");
const DIGICERT_ROOT_G2_DER: &[u8] = include_bytes!("digicert_global_root_g2.bin");

fn microsoft_signed_avd_rdp() -> String {
    let sig_b64 = STANDARD.encode(AVD_RDP_SIGNATURE_BIN);
    format!(
        concat!(
            "gatewayusagemethod:i:1\r\n",
            "gatewayprofileusagemethod:i:1\r\n",
            "authentication level:i:1\r\n",
            "gatewaybrokeringtype:i:1\r\n",
            "wvd endpoint pool:s:47ddc51c-2ce6-466a-8ee9-453f7b2072ee\r\n",
            "geo:s:CA__False\r\n",
            "armpath:s:/subscriptions/6fe9553e-226e-4185-9cc2-da3c31036ca8/resourcegroups/AVD/providers/Microsoft.DesktopVirtualization/hostpools/AvdLab\r\n",
            "aadtenantid:s:4c5a5ec1-9ac5-4612-9b4c-6bed178bb65a\r\n",
            "full address:s:rdgateway-r1.wvd.microsoft.com\r\n",
            "alternate full address:s:rdgateway-r1.wvd.microsoft.com\r\n",
            "diagnosticserviceurl:s:https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/DiagnosticEvents/v1\r\n",
            "hubdiscoverygeourl:s:https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/hubdiscovery?resourceId=47ddc51c-2ce6-466a-8ee9-453f7b2072ee\r\n",
            "resourceprovider:s:arm\r\n",
            "gatewayhostname:s:afdfp-rdgateway-r1.wvd.microsoft.com:443\r\n",
            "loadbalanceinfo:s:mth://localhost/4912db3f-ea4f-4c4f-bb20-5b6f6303c2c7/47ddc51c-2ce6-466a-8ee9-453f7b2072ee\r\n",
            "workspace id:s:ada6b91e-20af-4bb3-aea1-111328fd1811\r\n",
            "promptcredentialonce:i:1\r\n",
            "gatewaycredentialssource:i:0\r\n",
            "remoteapplicationprogram:s:||20842633-c909-4aeb-3199-08db09107d3b\r\n",
            "remotedesktopname:s:SessionDesktop\r\n",
            "remoteapplicationmode:i:0\r\n",
            "drivestoredirect:s:*\r\n",
            "audiomode:i:0\r\n",
            "videoplaybackmode:i:1\r\n",
            "redirectclipboard:i:1\r\n",
            "redirectprinters:i:1\r\n",
            "devicestoredirect:s:*\r\n",
            "redirectcomports:i:1\r\n",
            "redirectsmartcards:i:1\r\n",
            "usbdevicestoredirect:s:*\r\n",
            "enablecredsspsupport:i:1\r\n",
            "redirectwebauthn:i:1\r\n",
            "targetisaadjoined:i:1\r\n",
            "use multimon:i:1\r\n",
            "enablerdsaadauth:i:1\r\n",
            "signscope:s:Full Address,Alternate Full Address,EnableCredSspSupport,GatewayHostname,GatewayUsageMethod,GatewayProfileUsageMethod,GatewayCredentialsSource,PromptCredentialOnce,RemoteApplicationProgram,RemoteApplicationMode,RemoteDesktopName,Authentication Level,AudioMode,RedirectPrinters,RedirectCOMPorts,RedirectSmartCards,RedirectClipboard,DevicesToRedirect,DrivesToRedirect,LoadBalanceInfo,ArmPath,Geo,EnableRdsAadAuth,RedirectWebAuthn,AADTenantId,GatewayBrokeringtype,DiagnosticServiceUrl,ResourceProvider,Wvd Endpoint Pool,Workspace Id\r\n",
            "signature:s:{}\r\n",
        ),
        sig_b64
    )
}

fn digicert_root_g2_der() -> Vec<u8> {
    DIGICERT_ROOT_G2_DER.to_vec()
}

// oracle: spec https://learn.microsoft.com/windows-app/direct-launch-urls
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

// oracle: spec RFC 3986 §3.3
#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[case::slash_path("/path")]
#[case::query_injection("id?token=secret")]
#[case::fragment_injection("id#fragment")]
#[case::whitespace("a b")]
#[case::backslash("a\\b")]
#[case::crlf("\r\n")]
fn launch_urls_reject_missing_or_injected_ids(#[case] invalid: &str) {
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

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[test]
fn decode_discovery_and_continuation() {
    let body = format!(
        r#"{{"value":[{{"id":"{PC_ID}","displayName":"Demo-1","servicePlanType":"futurePlan",
        "provisioningType":"dedicated","newProperty":true}}],
        "@odata.nextLink":"{CLOUD_PCS_URL}?$skiptoken=opaque%2Bvalue"}}"#
    );
    let page = CloudPcPage::decode(body.as_bytes()).unwrap();
    assert_eq!(page.cloud_pcs.len(), 1);
    let pc = &page.cloud_pcs[0];
    assert_eq!(pc.id, PC_ID);
    assert_eq!(pc.display_name.as_deref(), Some("Demo-1"));
    assert_eq!(pc.service_plan_type.as_deref(), Some("futurePlan"));
    assert_eq!(pc.provisioning_type.as_deref(), Some("dedicated"));
    assert_eq!(
        pc.launch_url(None).unwrap(),
        format!("https://windows.cloud.microsoft/webclient/ent/{PC_ID}")
    );
    assert_eq!(
        page.next_link.as_deref(),
        Some(format!("{CLOUD_PCS_URL}?$skiptoken=opaque%2Bvalue").as_str())
    );
    let empty = CloudPcPage::decode(br#"{"value":[]}"#).unwrap();
    assert!(empty.cloud_pcs.is_empty());
    assert!(empty.next_link.is_none());
    let nullable = CloudPcPage::decode(br#"{"value":[{"id":"opaque","displayName":null}]}"#).unwrap();
    assert!(nullable.cloud_pcs[0].display_name.is_none());
    assert!(nullable.cloud_pcs[0].service_plan_type.is_none());
}

// oracle: spec https://learn.microsoft.com/graph/api/cloudpc-retrievecloudpclaunchdetail?view=graph-rest-1.0
#[test]
fn launch_detail_uses_documented_graph_resource_route() {
    assert_eq!(
        launch_detail_request_url(PC_ID).unwrap().as_str(),
        format!("{CLOUD_PCS_URL}/{PC_ID}/retrieveCloudPcLaunchDetail")
    );
}

// oracle: spec RFC 3986 §3.3
#[rstest]
#[case::empty("")]
#[case::parent_dir("../other")]
#[case::subpath("id/path")]
#[case::query("id?access_token=secret")]
#[case::fragment("id#fragment")]
#[case::space("a b")]
fn launch_detail_rejects_invalid_cloud_pc_id(#[case] invalid: &str) {
    assert!(matches!(
        launch_detail_request_url(invalid),
        Err(Error::InvalidInput("Cloud PC ID"))
    ));
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[rstest]
#[case::insecure_http("http://graph.microsoft.com/v1.0/me/cloudPCs")]
#[case::foreign_host("https://attacker.invalid/v1.0/me/cloudPCs")]
#[case::subdomain_spoof("https://graph.microsoft.com.attacker.invalid/v1.0/me/cloudPCs")]
#[case::non_default_port("https://graph.microsoft.com:444/v1.0/me/cloudPCs")]
#[case::userinfo("https://user@graph.microsoft.com/v1.0/me/cloudPCs")]
#[case::wrong_collection_path("https://graph.microsoft.com/v1.0/users")]
#[case::url_fragment("https://graph.microsoft.com/v1.0/me/cloudPCs#fragment")]
#[case::beta_endpoint("https://graph.microsoft.com/beta/me/cloudPCs")]
#[case::relative_url("/v1.0/me/cloudPCs?$skiptoken=x")]
fn continuations_cannot_change_bearer_destination(#[case] invalid: &str) {
    assert!(matches!(continuation_url(invalid), Err(Error::InvalidContinuation)));
    let body = format!(r#"{{"value":[],"@odata.nextLink":"{invalid}"}}"#);
    assert!(matches!(
        CloudPcPage::decode(body.as_bytes()),
        Err(Error::InvalidContinuation)
    ));
    let next = format!("{CLOUD_PCS_URL}?$skiptoken=opaque%2Bvalue");
    assert_eq!(continuation_url(&next).unwrap().as_str(), next);
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[rstest]
#[case::empty_body(&b""[..])]
#[case::non_json(&b"not JSON"[..])]
#[case::missing_value_array(&b"{}"[..])]
#[case::null_value_array(&br#"{"value":null}"#[..])]
#[case::missing_pc_id(&br#"{"value":[{"displayName":"missing ID"}]}"#[..])]
#[case::integer_pc_id(&br#"{"value":[{"id":42}]}"#[..])]
fn invalid_discovery_is_not_an_empty_success(#[case] invalid: &[u8]) {
    assert!(matches!(CloudPcPage::decode(invalid), Err(Error::InvalidResponse(_))));
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[test]
fn page_size_limit_exact_boundary() {
    let mut body = br#"{"value":[]}"#.to_vec();
    body.resize(MAX_PAGE_BYTES - 1, b' ');
    assert!(CloudPcPage::decode(&body).is_ok());
    body.push(b' ');
    assert!(CloudPcPage::decode(&body).is_ok());
    body.push(b' ');
    assert!(matches!(CloudPcPage::decode(&body), Err(Error::ResponseTooLarge)));
}

// oracle: spec RFC 6750 §2.1
#[tokio::test]
async fn invalid_tokens_and_continuations_fail_before_network() {
    let client = CloudPcClient::new().unwrap();
    // oracle: spec RFC 6750 §2.1
    for token in ["", "not a token", "secret\r\nInjected: header"] {
        assert!(matches!(
            client.list_page(token, None).await,
            Err(Error::InvalidInput("Graph bearer token"))
        ));
    }
    // oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
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
    // oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
    assert!(matches!(
        client.retrieve_launch_detail("token", "id/path").await,
        Err(Error::InvalidInput("Cloud PC ID"))
    ));
    // oracle: spec RFC 6750 §2.1
    assert!(matches!(
        client.retrieve_launch_detail("not a token", PC_ID).await,
        Err(Error::InvalidInput("Graph bearer token"))
    ));
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[rstest]
#[case::attacker_domain("https://attacker.invalid/api/arm/feeddiscovery")]
#[case::subdomain_spoof("https://wvd.microsoft.com.attacker.invalid/api/arm/feeddiscovery")]
#[case::loopback_ip("https://127.0.0.1/api/arm/feeddiscovery")]
#[case::plaintext_http("http://www.wvd.microsoft.com/api/arm/feeddiscovery")]
#[case::non_default_port("https://www.wvd.microsoft.com:444/api/arm/feeddiscovery")]
#[case::userinfo("https://user@www.wvd.microsoft.com/api/arm/feeddiscovery")]
#[case::fragment("https://www.wvd.microsoft.com/api/arm/feeddiscovery#frag")]
#[tokio::test]
async fn avd_authenticated_requests_reject_attacker_controlled_https_origins(#[case] untrusted: &str) {
    let client = CloudPcClient::new().unwrap();
    let url = Url::parse(untrusted).unwrap();
    assert!(matches!(
        client.discover_avd_feeds(url.clone(), "token").await,
        Err(Error::InvalidAvdUrl)
    ));
    assert!(matches!(
        client.fetch_avd_workspace_feed(url.clone(), "token").await,
        Err(Error::InvalidAvdUrl)
    ));
    assert!(matches!(
        client.download_avd_rdp_file(url, "token").await,
        Err(Error::InvalidAvdUrl)
    ));
}

// oracle: spec RFC 6750 §2.1
#[tokio::test]
async fn http_discovery_sends_bearer_and_decodes_page() {
    let (url, request) = serve(http_response("200 OK", r#"{"value":[{"id":"demo"}]}"#)).await;
    let page = test_client().fetch_page(url, "test-token").await.unwrap();
    assert_eq!(page.cloud_pcs[0].id, "demo");
    let request = request.await.unwrap().to_ascii_lowercase();
    assert!(request.starts_with("get /v1.0/me/cloudpcs http/1.1\r\n"));
    assert!(request.contains("\r\nauthorization: bearer test-token\r\n"));
    assert!(request.contains("\r\naccept: application/json\r\n"));
    assert!(!request.contains("?"));
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn http_launch_detail_returns_redacted_service_browser_url() {
    let response = format!(
        r#"{{"cloudPcId":"{PC_ID}","cloudPcLaunchUrl":"https://rdweb-r0.wvdselfhost.microsoft.com/api/arm/weblaunch/tenants/demo/resources/demo?ticket=opaque-secret","windows365SwitchCompatible":false,"windows365SwitchCompatibilityFailureReasonType":"osVersionNotSupported"}}"#
    );
    let (url, request) = serve(http_response("200 OK", &response)).await;
    let detail = test_client()
        .fetch_launch_detail(url, "test-token", PC_ID)
        .await
        .unwrap();
    assert_eq!(detail.cloud_pc_id(), PC_ID);
    assert_eq!(
        detail.launch_url().as_str(),
        "https://rdweb-r0.wvdselfhost.microsoft.com/api/arm/weblaunch/tenants/demo/resources/demo?ticket=opaque-secret"
    );
    assert!(!detail.windows365_switch_compatible());
    assert_eq!(
        detail.windows365_switch_compatibility_failure_reason_type(),
        Some("osVersionNotSupported")
    );
    let debug = format!("{detail:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("opaque-secret"));
    let request = request.await.unwrap().to_ascii_lowercase();
    assert!(request.contains("\r\nauthorization: bearer test-token\r\n"));
    assert!(!request.contains("opaque-secret"));
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn launch_detail_rejects_mismatched_or_untrusted_service_urls() {
    let wrong_id = r#"{"cloudPcId":"another-id","cloudPcLaunchUrl":"https://windows.cloud.microsoft/","windows365SwitchCompatible":true}"#;
    let (url, request) = serve(http_response("200 OK", wrong_id)).await;
    assert!(matches!(
        test_client().fetch_launch_detail(url, "token", PC_ID).await,
        Err(Error::LaunchDetailMismatch)
    ));
    request.await.unwrap();

    for launch_url in [
        "http://rdweb.example/launch",
        "https://user@rdweb.example/launch",
        "https://rdweb.example:444/launch",
        "not a URL",
    ] {
        let body =
            format!(r#"{{"cloudPcId":"{PC_ID}","cloudPcLaunchUrl":"{launch_url}","windows365SwitchCompatible":true}}"#);
        let (url, request) = serve(http_response("200 OK", &body)).await;
        assert!(matches!(
            test_client().fetch_launch_detail(url, "token", PC_ID).await,
            Err(Error::InvalidLaunchUrl)
        ));
        request.await.unwrap();
    }
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn launch_detail_rejects_missing_contract_fields_and_service_errors() {
    for body in ["{}", r#"{"cloudPcId":"demo","cloudPcLaunchUrl":"https://example.com"}"#] {
        let (url, request) = serve(http_response("200 OK", body)).await;
        assert!(matches!(
            test_client().fetch_launch_detail(url, "token", "demo").await,
            Err(Error::InvalidResponse(_))
        ));
        request.await.unwrap();
    }
    let (url, request) = serve(http_response("403 Forbidden", "secret error")).await;
    let error = test_client()
        .fetch_launch_detail(url, "token", PC_ID)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::HttpStatus(403)));
    assert!(!format!("{error:?}").contains("secret error"));
    request.await.unwrap();
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn http_status_and_redirects_are_not_success() {
    for (expected_code, status) in [
        (401, "401 Unauthorized"),
        (403, "403 Forbidden"),
        (429, "429 Too Many Requests"),
        (503, "503 Unavailable"),
    ] {
        let (url, request) = serve(http_response(status, "sensitive service error")).await;
        let error = test_client().fetch_page(url, "test-token").await.unwrap_err();
        assert!(matches!(error, Error::HttpStatus(code) if code == expected_code));
        assert!(!format!("{error:?}").contains("sensitive"));
        request.await.unwrap();
    }
    let (url, request) =
        serve(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/token-leak\r\nContent-Length: 0\r\n\r\n".to_vec())
            .await;
    assert!(matches!(
        test_client().fetch_page(url, "test-token").await,
        Err(Error::HttpStatus(302))
    ));
    request.await.unwrap();
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn oversized_streams_and_invalid_http_bodies_fail() {
    let (url, request) =
        serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_PAGE_BYTES + 1).into_bytes()).await;
    assert!(matches!(
        test_client().fetch_page(url, "test-token").await,
        Err(Error::ResponseTooLarge)
    ));
    request.await.unwrap();
    let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    response.resize(response.len() + MAX_PAGE_BYTES + 1, b' ');
    let (url, request) = serve(response).await;
    assert!(matches!(
        test_client().fetch_page(url, "test-token").await,
        Err(Error::ResponseTooLarge)
    ));
    request.await.unwrap();
    let (url, request) = serve(http_response("200 OK", "{}")).await;
    assert!(matches!(
        test_client().fetch_page(url, "test-token").await,
        Err(Error::InvalidResponse(_))
    ));
    request.await.unwrap();
}

// oracle: spec crates/ironrdp-client/README.md §Cloud discovery and browser handoff
#[tokio::test]
async fn transport_errors_do_not_expose_continuation_urls() {
    let (url, request) = serve(b"not an HTTP response".to_vec()).await;
    let error = test_client().fetch_page(url, "test-token").await.unwrap_err();
    assert!(matches!(&error, Error::Transport(source) if source.url().is_none()));
    assert!(!format!("{error:?}").contains("test-token"));
    request.await.unwrap();
}

// oracle: spec MS-TSWP §2.2.2
#[test]
fn avd_feed_discovery_and_workspace_xml_parsing() {
    let url = AvdCloudEnvironment::Public.feed_discovery_url(Some(TENANT_ID)).unwrap();
    assert_eq!(
        url.as_str(),
        format!("https://www.wvd.microsoft.com/api/arm/feeddiscovery?aadtenant={TENANT_ID}")
    );
    assert_eq!(
        AvdCloudEnvironment::UsGovernment
            .feed_discovery_url(None)
            .unwrap()
            .as_str(),
        "https://www.wvd.azure.us/api/arm/feeddiscovery"
    );
    assert_eq!(
        AvdCloudEnvironment::China.feed_discovery_url(None).unwrap().as_str(),
        "https://www.wvd.azure.cn/api/arm/feeddiscovery"
    );

    let feed_discovery_xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<FeedDiscovery>
  <Publisher TenantId="{WORKSPACE_ID}"
             TenantDisplayName="TestWS"
             FeedURL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/{WORKSPACE_ID}/webfeed?geo=CA&amp;showInFeedMonikers=CloudPC"
             HubDiscoveryGeoURL="https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/hubdiscovery?resourceId={PC_ID}"
             DiagnosticServiceURL="https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/DiagnosticEvents/v1"
             ActivityHint="ms-wvd-hp:{PC_ID}" />
</FeedDiscovery>"#
    );
    let subs = parse_avd_feed_discovery_xml(&feed_discovery_xml).unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].tenant_id, WORKSPACE_ID);
    assert_eq!(subs[0].tenant_display_name.as_deref(), Some("TestWS"));
    assert_eq!(
        subs[0].feed_url.as_str(),
        format!(
            "https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/{WORKSPACE_ID}/webfeed?geo=CA&showInFeedMonikers=CloudPC"
        )
    );
    assert!(subs[0].hub_discovery_geo_url.is_some());
    assert!(subs[0].diagnostic_service_url.is_some());

    let ws_xml = format!(
        r#"<ResourceCollection><Publisher ID="{WORKSPACE_ID}" Name="TestWS"><Resources><Resource ID="{PC_ID}" Title="SessionDesktop" Type="Desktop" DeviceState="Available"><HostingTerminalServers><HostingTerminalServer><ResourceFile FileExtension=".rdp" URL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/{WORKSPACE_ID}/rdps/{PC_ID}.rdp?geo=CA" ETag="v1" /></HostingTerminalServer></HostingTerminalServers></Resource></Resources></Publisher></ResourceCollection>"#
    );
    let ws = parse_avd_workspace_feed_xml(&ws_xml).unwrap();
    assert_eq!(ws.workspace_id.as_deref(), Some(WORKSPACE_ID));
    assert_eq!(ws.workspace_name.as_deref(), Some("TestWS"));
    assert_eq!(ws.resources.len(), 1);
    assert_eq!(ws.resources[0].kind, AvdResourceKind::Desktop);
    assert_eq!(ws.resources[0].id, PC_ID);
    assert_eq!(ws.resources[0].device_state.as_deref(), Some("Available"));
    assert_eq!(ws.resources[0].rdp_etag.as_deref(), Some("v1"));

    // Untrusted FeedURL or ResourceFile hosts in XML are rejected.
    let untrusted_feed_xml = format!(
        r#"<FeedDiscovery><Publisher TenantId="{WORKSPACE_ID}" FeedURL="https://attacker.invalid/webfeed" /></FeedDiscovery>"#
    );
    assert!(matches!(
        parse_avd_feed_discovery_xml(&untrusted_feed_xml),
        Err(Error::InvalidXml("untrusted FeedURL"))
    ));
    let untrusted_res_xml = format!(
        r#"<ResourceCollection><Publisher ID="{WORKSPACE_ID}"><Resources><Resource ID="{PC_ID}" Title="D" Type="Desktop"><HostingTerminalServers><HostingTerminalServer><ResourceFile FileExtension=".rdp" URL="https://attacker.invalid/evil.rdp" /></HostingTerminalServer></HostingTerminalServers></Resource></Resources></Publisher></ResourceCollection>"#
    );
    assert!(matches!(
        parse_avd_workspace_feed_xml(&untrusted_res_xml),
        Err(Error::InvalidXml("untrusted ResourceFile URL"))
    ));

    // Relative ResourceFile URL (as in MS-TSWP examples) resolves against the AVD origin.
    let relative_res_xml = format!(
        r#"<ResourceCollection><Publisher ID="{WORKSPACE_ID}"><Resources><Resource ID="{PC_ID}" Title="D" Type="Desktop"><HostingTerminalServers><HostingTerminalServer><ResourceFile FileExtension=".rdp" URL="/RDWeb/Pages/rdp/{PC_ID}.rdp" /></HostingTerminalServer></HostingTerminalServers></Resource></Resources></Publisher></ResourceCollection>"#
    );
    let relative_ws = parse_avd_workspace_feed_xml(&relative_res_xml).unwrap();
    assert_eq!(
        relative_ws.resources[0].rdp_file_url.as_ref().unwrap().as_str(),
        format!("https://www.wvd.microsoft.com/RDWeb/Pages/rdp/{PC_ID}.rdp")
    );
}

// oracle: spec MS-TSWP §2.2.2
#[test]
fn xml_parser_handles_quoted_gt_comments_and_utf8_element_names() {
    let utf8_tag = "Resourc\u{e9}";
    let xml = format!(
        r#"<ResourceCollection>
  <!-- <Resource ID="commented-out" Title="Ignored" Type="Desktop" /> -->
  <{utf8_tag} />
  <Publisher ID="{WORKSPACE_ID}" Name="A &gt; B &amp;lt; C">
    <Resources>
      <Resource ID="{PC_ID}" Title="A > B" Type="RemoteApp" />
    </Resources>
  </Publisher>
</ResourceCollection>"#
    );
    let ws = parse_avd_workspace_feed_xml(&xml).unwrap();
    assert_eq!(ws.workspace_name.as_deref(), Some("A > B &lt; C"));
    assert_eq!(ws.resources.len(), 1);
    assert_eq!(ws.resources[0].id, PC_ID);
    assert_eq!(ws.resources[0].title, "A > B");
    assert_eq!(ws.resources[0].kind, AvdResourceKind::RemoteApp);

    let utf8_only = parse_avd_workspace_feed_xml("<ResourceCollection><Resourc\u{e9}/></ResourceCollection>").unwrap();
    assert!(utf8_only.resources.is_empty());

    // Unsigned embedded <RDPFile> in workspace feed XML must fail signature verification.
    let unsigned_embedded = format!(
        r#"<ResourceCollection><Publisher ID="{WORKSPACE_ID}"><Resources><Resource ID="{PC_ID}" Title="D" Type="Desktop"><RDPFile>full address:s:attacker.invalid</RDPFile></Resource></Resources></Publisher></ResourceCollection>"#
    );
    assert!(matches!(
        parse_avd_workspace_feed_xml(&unsigned_embedded),
        Err(Error::InvalidRdpSignature(_))
    ));
}

// oracle: reference mstscax.dll 10.0.26100.9711 VerifySensitiveSettings
#[test]
fn mstsc_sensitive_settings_matches_activex_oracle() {
    assert_eq!(MAX_SIGN_SCOPE_CHARS, 0x800);
    assert_eq!(MSTSC_SENSITIVE_SETTINGS.len(), 48);
    assert_eq!(MSTSC_SENSITIVE_SETTINGS[0], "Full Address");
    assert_eq!(MSTSC_SENSITIVE_SETTINGS[6], "EnableCredSspSupport");
    assert_eq!(MSTSC_SENSITIVE_SETTINGS[43], "KDCProxyName");
    assert_eq!(MSTSC_SENSITIVE_SETTINGS[44], "EnableRdsAadAuth");
    assert_eq!(MSTSC_SENSITIVE_SETTINGS[47], "allowed security protocols");
}

// oracle: reference crates/ironrdp-testsuite-extra/tests/client/avd_rdp_signature.bin
#[test]
fn verify_microsoft_signed_avd_rdp_file_and_detect_tampering() {
    let signed_rdp = microsoft_signed_avd_rdp();
    let root_der = digicert_root_g2_der();
    let anchors = [root_der.as_slice()];
    // 2026-10-26T18:00:00Z is within the leaf certificate validity window (2026-10-12..2027-01-20).
    let valid_time = Duration::from_secs(1_793_047_200);

    let verified = verify_rdp_signature_at(valid_time, &signed_rdp, &anchors).unwrap();
    assert_eq!(verified.signed_scope.len(), 30);
    assert!(verified.signer_subject.contains("rdp.wvd.microsoft.com"));
    assert!(!verified.signer_certificate_der.is_empty());

    // Tampering with a signed property must fail signature verification.
    let tampered = signed_rdp.replace(
        "full address:s:rdgateway-r1.wvd.microsoft.com",
        "full address:s:attacker.example.com",
    );
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &tampered, &anchors),
        Err(Error::InvalidRdpSignature("signature verification failed"))
    ));

    // Unsigned enablecredsspsupport downgrade (present in file but omitted from signscope) must fail.
    let without_credssp_in_scope = signed_rdp.replace(
        "Full Address,Alternate Full Address,EnableCredSspSupport,",
        "Full Address,Alternate Full Address,",
    );
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &without_credssp_in_scope, &anchors),
        Err(Error::InvalidRdpSignature(
            "sensitive .rdp setting is not covered by signscope"
        ))
    ));

    // Prepending or appending a duplicate property (even with different case) must fail.
    let duplicate_host = format!("Full Address:s:attacker.example.com\r\n{signed_rdp}");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &duplicate_host, &anchors),
        Err(Error::InvalidRdpSignature("duplicate .rdp property"))
    ));
    let duplicate_credssp = format!("{signed_rdp}enablecredsspsupport:i:0\r\n");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &duplicate_credssp, &anchors),
        Err(Error::InvalidRdpSignature("duplicate .rdp property"))
    ));
}

// oracle: reference mstscax.dll 10.0.26100.9711 VerifySensitiveSettings
#[rstest]
#[case::alternate_shell("alternate shell:s:cmd.exe")]
#[case::server_port("server port:i:3390")]
#[case::kdc_proxy_name("kdcproxyname:s:attacker.invalid")]
#[case::kdc_proxy_url("kdcproxyurl:s:https://attacker.invalid/KdcProxy")]
#[case::use_redirection_server_name("use redirection server name:i:1")]
#[case::negotiate_security_layer("negotiate security layer:i:0")]
#[case::remote_app_cmdline("remoteapplicationcmdline:s:--evil")]
#[case::admin_session("administrative session:i:1")]
#[case::ironrdp_tls("ironrdp_tls:i:0")]
#[case::ironrdp_named_pipe("ironrdp_named_pipe:s:\\\\.\\pipe\\evil")]
#[case::ironrdp_rdcleanpath_url("ironrdp_rdcleanpathurl:s:https://attacker.invalid/rdcleanpath")]
#[case::ironrdp_rdcleanpath_token("ironrdp_rdcleanpathtoken:s:secret")]
#[case::ironrdp_dvc_plugin("ironrdp_dvcplugin:s:evil.dll")]
fn unsigned_sensitive_setting_is_rejected(#[case] unsigned_sensitive: &str) {
    let signed_rdp = microsoft_signed_avd_rdp();
    let root_der = digicert_root_g2_der();
    let anchors = [root_der.as_slice()];
    let valid_time = Duration::from_secs(1_793_047_200);
    let with_unsigned = format!("{signed_rdp}{unsigned_sensitive}\r\n");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &with_unsigned, &anchors),
        Err(Error::InvalidRdpSignature(
            "sensitive .rdp setting is not covered by signscope"
        ))
    ));
}

// oracle: reference crates/ironrdp-testsuite-extra/tests/client/digicert_global_root_g2.bin
#[test]
fn certificate_chain_validation_rejects_untrusted_incomplete_or_expired_paths() {
    let signed_rdp = microsoft_signed_avd_rdp();
    let root_der = digicert_root_g2_der();
    let anchors = [root_der.as_slice()];
    let valid_time = Duration::from_secs(1_793_047_200);

    // Expired timestamp (2030-01-01) and not-yet-valid timestamp (2020-01-01) must fail.
    assert!(matches!(
        verify_rdp_signature_at(Duration::from_secs(1_893_456_000), &signed_rdp, &anchors),
        Err(Error::InvalidRdpSignature(
            "certificate in PKCS#7 chain is expired or not yet valid"
        ))
    ));
    assert!(matches!(
        verify_rdp_signature_at(Duration::from_secs(1_577_836_800), &signed_rdp, &anchors),
        Err(Error::InvalidRdpSignature(
            "certificate in PKCS#7 chain is expired or not yet valid"
        ))
    ));

    // Verifying against an unrelated trust anchor (using the leaf cert itself, which is not a CA, as the only anchor) must fail.
    let verified = verify_rdp_signature_at(valid_time, &signed_rdp, &anchors).unwrap();
    let leaf_as_anchor = [verified.signer_certificate_der.as_slice()];
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &signed_rdp, &leaf_as_anchor),
        Err(Error::InvalidRdpSignature(
            "signer certificate chain is not anchored in a trusted root"
        ))
    ));

    // Invalid 12-byte RDP signature headers (bad version, bad cert type, missing flag bit 0) must fail.
    let bad_version = signed_rdp.replace("signature:s:AQABAAEAAABsGwAA", "signature:s:AgABAAEAAABsGwAA");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &bad_version, &anchors),
        Err(Error::InvalidRdpSignature("invalid signature header"))
    ));
    let bad_type = signed_rdp.replace("signature:s:AQABAAEAAABsGwAA", "signature:s:AQACAAEAAABsGwAA");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &bad_type, &anchors),
        Err(Error::InvalidRdpSignature("invalid signature header"))
    ));
    let bad_flags = signed_rdp.replace("signature:s:AQABAAEAAABsGwAA", "signature:s:AQABAAAAAABsGwAA");
    assert!(matches!(
        verify_rdp_signature_at(valid_time, &bad_flags, &anchors),
        Err(Error::InvalidRdpSignature("invalid signature header"))
    ));

    // Backtracking across candidate issuers: insert a dead-end cross-signed intermediate
    // (same Subject and SPKI as Cert 1, so it verifies Cert 2's signature, but its Issuer DN
    // points to a non-existent root) before the valid Cert 1 in the PKCS#7 certificate bag.
    let raw_sig = include_bytes!("avd_rdp_signature.bin");
    let cert1_len: u16 = 1421;
    let mut dead_end_cert1 = raw_sig[984..984 + usize::from(cert1_len)].to_vec();
    dead_end_cert1[122..122 + 23].copy_from_slice(b"DigiCert Global Root XX");
    let mut patched_sig = Vec::with_capacity(raw_sig.len() + dead_end_cert1.len());
    patched_sig.extend_from_slice(&raw_sig[..984]);
    patched_sig.extend_from_slice(&dead_end_cert1);
    patched_sig.extend_from_slice(&raw_sig[984..]);
    let new_declared_len =
        u32::from_le_bytes([raw_sig[8], raw_sig[9], raw_sig[10], raw_sig[11]]) + u32::from(cert1_len);
    patched_sig[8..12].copy_from_slice(&new_declared_len.to_le_bytes());
    for off in [12usize, 27, 31, 66] {
        let old_len = u16::from_be_bytes([raw_sig[off + 2], raw_sig[off + 3]]);
        patched_sig[off + 2..off + 4].copy_from_slice(&(old_len + cert1_len).to_be_bytes());
    }
    let orig_sig_b64 = STANDARD.encode(raw_sig);
    let patched_sig_b64 = STANDARD.encode(&patched_sig);
    let cross_signed_rdp = signed_rdp.replace(&orig_sig_b64, &patched_sig_b64);
    // oracle: spec RFC 5280 §6.1
    let cross_verified = verify_rdp_signature_at(valid_time, &cross_signed_rdp, &anchors).unwrap();
    assert!(cross_verified.signer_subject.contains("rdp.wvd.microsoft.com"));
}

// oracle: spec RFC 5652 §5.3
#[test]
fn cms_signed_attrs_requires_exact_message_digest_attribute() {
    let digest = [0x42u8; 32];

    // Valid contentType (1.2.840.113549.1.9.3 = id-data) + messageDigest (1.2.840.113549.1.9.4 = digest)
    let mut valid_attrs = Vec::new();
    // Attribute 1: contentType -> id-data
    valid_attrs.extend_from_slice(&[
        0x30, 0x18, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03, 0x31, 0x0b, 0x06, 0x09, 0x2a,
        0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01,
    ]);
    // Attribute 2: messageDigest -> OCTET STRING (32 bytes)
    let mut md_attr = vec![
        0x30, 0x2f, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04, 0x31, 0x22, 0x04, 0x20,
    ];
    md_attr.extend_from_slice(&digest);
    valid_attrs.extend_from_slice(&md_attr);
    assert!(verify_signed_attrs_for_test(&valid_attrs, &digest).is_ok());

    // Missing contentType attribute (RFC 5652 §11.1 requires both contentType and messageDigest) must fail.
    assert!(matches!(
        verify_signed_attrs_for_test(&md_attr, &digest),
        Err(Error::InvalidRdpSignature(
            "PKCS#7 signedAttrs must contain exactly one contentType and one messageDigest"
        ))
    ));

    // Embedding the expected digest inside an unrelated attribute OID (e.g. 1.2.840.113549.1.9.99) without messageDigest must fail.
    let mut unrelated_attr = vec![
        0x30, 0x2f, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x63, 0x31, 0x22, 0x04, 0x20,
    ];
    unrelated_attr.extend_from_slice(&digest);
    assert!(matches!(
        verify_signed_attrs_for_test(&unrelated_attr, &digest),
        Err(Error::InvalidRdpSignature(
            "PKCS#7 signedAttrs must contain exactly one contentType and one messageDigest"
        ))
    ));

    // Duplicate messageDigest attributes must fail.
    let mut duplicate_md = valid_attrs.clone();
    duplicate_md.extend_from_slice(&md_attr);
    assert!(matches!(
        verify_signed_attrs_for_test(&duplicate_md, &digest),
        Err(Error::InvalidRdpSignature(
            "PKCS#7 signedAttrs must contain exactly one contentType and one messageDigest"
        ))
    ));

    // Wrong digest value in messageDigest must fail.
    let wrong_digest = [0x99u8; 32];
    assert!(matches!(
        verify_signed_attrs_for_test(&valid_attrs, &wrong_digest),
        Err(Error::InvalidRdpSignature("PKCS#7 messageDigest attribute mismatch"))
    ));
}

// oracle: reference crates/ironrdp-testsuite-extra/tests/client/avd_rdp_signature.bin
#[tokio::test]
async fn download_avd_rdp_file_decodes_utf16le_with_bom_and_verifies() {
    let signed_rdp = microsoft_signed_avd_rdp();
    let root_der = digicert_root_g2_der();
    let anchors = [root_der.as_slice()];
    let valid_time = Duration::from_secs(1_793_047_200);

    let mut utf16le_body = vec![0xFF, 0xFE];
    for unit in signed_rdp.encode_utf16() {
        utf16le_body.extend_from_slice(&unit.to_le_bytes());
    }
    let mut raw_http = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-rdp\r\nContent-Length: {}\r\n\r\n",
        utf16le_body.len()
    )
    .into_bytes();
    raw_http.extend_from_slice(&utf16le_body);

    let (url, request) = serve(raw_http).await;
    let (rdp_text, verified) = test_client()
        .download_avd_rdp_file_at(Some(valid_time), url, "test-token", Some(&anchors))
        .await
        .unwrap();
    assert!(rdp_text.contains("full address:s:rdgateway-r1.wvd.microsoft.com\r\n"));
    // Non-sensitive properties (such as `use multimon:i:1`) are preserved in the returned canonical document.
    assert!(rdp_text.contains("use multimon:i:1\r\n"));
    assert_eq!(verified.signed_scope.len(), 30);
    let req_str = request.await.unwrap().to_ascii_lowercase();
    assert!(req_str.contains("\r\nauthorization: bearer test-token\r\n"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    // oracle: reference mstscax.dll 10.0.26100.9711 CRdpSettingsStore case-insensitive key lookup and line-order-independent signscope canonicalization
    #[test]
    fn canonicalization_is_invariant_under_line_order_case_and_line_endings(
        reverse_order in any::<bool>(),
        uppercase_keys in any::<bool>(),
        use_lf_only in any::<bool>(),
        leading_spaces in 0usize..3,
    ) {
        let signed_rdp = microsoft_signed_avd_rdp();
        let root_der = digicert_root_g2_der();
        let anchors = [root_der.as_slice()];
        let valid_time = Duration::from_secs(1_793_047_200);
        let baseline = verify_rdp_signature_at(valid_time, &signed_rdp, &anchors).unwrap();

        let prefix = " ".repeat(leading_spaces);
        let mut lines: Vec<String> = signed_rdp
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let (key, rest) = line.split_once(':').unwrap();
                let transformed_key = if uppercase_keys && !key.eq_ignore_ascii_case("signscope") {
                    key.to_ascii_uppercase()
                } else {
                    key.to_owned()
                };
                format!("{prefix}{transformed_key}:{rest}")
            })
            .collect();
        if reverse_order {
            lines.reverse();
        }
        let sep = if use_lf_only { "\n" } else { "\r\n" };
        let mutated = format!("{}{sep}", lines.join(sep));

        let verified = verify_rdp_signature_at(valid_time, &mutated, &anchors).unwrap();
        prop_assert_eq!(verified, baseline);
    }

    // oracle: scaffold (untrusted multi-line XML, .rdp, and CMS signedAttrs inputs return structured errors without panicking; these parsers are not wired into cargo-fuzz)
    #[test]
    fn parsers_never_panic_on_arbitrary_malformed_inputs(
        raw in "[\\s\\S]{0,512}",
        raw_bytes in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let root_der = digicert_root_g2_der();
        let anchors = [root_der.as_slice()];
        let valid_time = Duration::from_secs(1_793_047_200);

        let _ = parse_avd_feed_discovery_xml(&raw);
        let _ = parse_avd_workspace_feed_xml(&raw);
        let _ = verify_rdp_signature_at(valid_time, &raw, &anchors);
        let _ = verify_signed_attrs_for_test(&raw_bytes, &[0u8; 32]);
    }
}

fn test_client() -> CloudPcClient {
    CloudPcClient::new_no_proxy().unwrap()
}

fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

async fn serve(response: Vec<u8>) -> (Url, JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = socket.read(&mut buf).await.unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&buf[..read]);
            assert!(request.len() < 16 * 1024);
        }
        socket.write_all(&response).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8(request).unwrap()
    });
    (Url::parse(&format!("http://{addr}/v1.0/me/cloudPCs")).unwrap(), task)
}
