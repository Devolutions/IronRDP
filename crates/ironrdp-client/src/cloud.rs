//! Cloud PC discovery and Windows App browser handoff, not native AVD reverse-connect.
//!
//! Authentication and browser launch belong to the caller.
//! Graph bearer tokens are not RDP session credentials.

use core::fmt;
use core::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use url::Url;

const CLOUD_PCS_URL: &str = "https://graph.microsoft.com/v1.0/me/cloudPCs";
const MAX_PAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    InvalidContinuation,
    Transport(reqwest::Error),
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidResponse(serde_json::Error),
    InvalidLaunchUrl,
    LaunchDetailMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(field) => write!(f, "invalid {field}"),
            Self::InvalidContinuation => f.write_str("invalid Cloud PC continuation URL"),
            Self::Transport(_) => f.write_str("Cloud PC service request failed"),
            Self::HttpStatus(status) => write!(f, "Cloud PC service returned HTTP {status}"),
            Self::ResponseTooLarge => f.write_str("Cloud PC response exceeds 1 MiB"),
            Self::InvalidResponse(_) => f.write_str("invalid Cloud PC response"),
            Self::InvalidLaunchUrl => f.write_str("invalid Cloud PC launch URL"),
            Self::LaunchDetailMismatch => f.write_str("Cloud PC launch detail does not match the requested device"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::InvalidResponse(error) => Some(error),
            _ => None,
        }
    }
}

/// The subset of [Cloud PC properties] needed for discovery and browser handoff.
///
/// Unknown properties are ignored; service-plan values are preserved as strings.
///
/// [Cloud PC properties]: https://learn.microsoft.com/graph/api/resources/cloudpc?view=graph-rest-1.0
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudPc {
    pub id: String,
    pub display_name: Option<String>,
    pub service_plan_type: Option<String>,
    pub provisioning_type: Option<String>,
}

impl CloudPc {
    /// Build a browser URL for a provisioned Enterprise or Flex Dedicated Cloud PC.
    ///
    /// This does not establish entitlement, provisioning status, or native RDP connectivity.
    pub fn launch_url(&self, tenant_id: Option<&str>) -> Result<String, Error> {
        windows365_launch_url(&self.id, tenant_id)
    }
}

/// One Graph response page; callers decide whether and when to request another page.
#[derive(Debug, Clone, Deserialize)]
pub struct CloudPcPage {
    #[serde(rename = "value")]
    pub cloud_pcs: Vec<CloudPc>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

/// Service-provided browser connection details for a signed-in user's Cloud PC.
///
/// The URL is intentionally omitted from `Debug` output because its query may contain
/// service-issued connection state.
#[derive(Clone)]
pub struct CloudPcLaunchDetail {
    cloud_pc_id: String,
    launch_url: Url,
    windows365_switch_compatible: bool,
    windows365_switch_compatibility_failure_reason_type: Option<String>,
}

impl CloudPcLaunchDetail {
    /// Graph identifier of the Cloud PC for this launch URL.
    pub fn cloud_pc_id(&self) -> &str {
        &self.cloud_pc_id
    }

    /// Service-provided HTTPS URL to open in a supported Windows App browser.
    ///
    /// This is a browser launch URL, not an RDP server address or reverse-connect
    /// transport endpoint.
    pub fn launch_url(&self) -> &Url {
        &self.launch_url
    }

    pub fn windows365_switch_compatible(&self) -> bool {
        self.windows365_switch_compatible
    }

    pub fn windows365_switch_compatibility_failure_reason_type(&self) -> Option<&str> {
        self.windows365_switch_compatibility_failure_reason_type.as_deref()
    }
}

impl fmt::Debug for CloudPcLaunchDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudPcLaunchDetail")
            .field("cloud_pc_id", &self.cloud_pc_id)
            .field("launch_url", &"[REDACTED]")
            .field("windows365_switch_compatible", &self.windows365_switch_compatible)
            .field(
                "windows365_switch_compatibility_failure_reason_type",
                &self.windows365_switch_compatibility_failure_reason_type,
            )
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCloudPcLaunchDetail {
    cloud_pc_id: String,
    cloud_pc_launch_url: String,
    windows365_switch_compatible: bool,
    windows365_switch_compatibility_failure_reason_type: Option<String>,
}

impl CloudPcPage {
    fn decode(body: &[u8]) -> Result<Self, Error> {
        if MAX_PAGE_BYTES < body.len() {
            return Err(Error::ResponseTooLarge);
        }
        let page: Self = serde_json::from_slice(body).map_err(Error::InvalidResponse)?;
        if let Some(next_link) = &page.next_link {
            continuation_url(next_link)?;
        }
        Ok(page)
    }
}

/// Global-cloud Graph client using platform trust roots and no HTTP redirects.
///
/// Requests have a 30-second timeout and a 1 MiB response limit.
/// Tokens are supplied per request and are not stored.
pub struct CloudPcClient {
    http: reqwest::Client,
}

impl CloudPcClient {
    pub fn new() -> Result<Self, Error> {
        let http = Self::client_builder()
            .build()
            .map_err(|error| Error::Transport(error.without_url()))?;
        Ok(Self { http })
    }

    fn client_builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
    }

    /// List [Cloud PCs attributed to the signed-in user].
    ///
    /// Supply a delegated Graph access token with `CloudPC.Read.All` permission.
    /// Use `None` for the first page and the previous page's `next_link` thereafter.
    /// Continuations must remain on the same HTTPS Graph collection.
    /// HTTP failures, including 401, 403, and 429, are returned without retries.
    ///
    /// [Cloud PCs attributed to the signed-in user]: https://learn.microsoft.com/graph/api/user-list-cloudpcs?view=graph-rest-1.0
    pub async fn list_page(&self, access_token: &str, next_link: Option<&str>) -> Result<CloudPcPage, Error> {
        let url = continuation_url(next_link.unwrap_or(CLOUD_PCS_URL))?;
        self.fetch_page(url, access_token).await
    }

    async fn fetch_page(&self, url: Url, access_token: &str) -> Result<CloudPcPage, Error> {
        let body = self.get(url, access_token).await?;
        CloudPcPage::decode(&body)
    }

    /// Retrieve the Graph-provided browser launch URL for a Cloud PC owned by the signed-in user.
    ///
    /// Requires delegated `CloudPC.Read.All` permission.
    /// Graph returns a `cloudPcLaunchUrl`; it does not return an RDP host endpoint, broker
    /// allocation result, feed connection file, or reverse-connect transport parameters.
    /// Open the returned URL in a supported Windows App browser.
    ///
    /// [Graph operation]: https://learn.microsoft.com/graph/api/cloudpc-retrievecloudpclaunchdetail?view=graph-rest-1.0
    pub async fn retrieve_launch_detail(
        &self,
        access_token: &str,
        cloud_pc_id: &str,
    ) -> Result<CloudPcLaunchDetail, Error> {
        let url = launch_detail_request_url(cloud_pc_id)?;
        self.fetch_launch_detail(url, access_token, cloud_pc_id).await
    }

    async fn fetch_launch_detail(
        &self,
        url: Url,
        access_token: &str,
        expected_cloud_pc_id: &str,
    ) -> Result<CloudPcLaunchDetail, Error> {
        let body = self.get(url, access_token).await?;
        let detail: RawCloudPcLaunchDetail = serde_json::from_slice(&body).map_err(Error::InvalidResponse)?;
        if detail.cloud_pc_id != expected_cloud_pc_id {
            return Err(Error::LaunchDetailMismatch);
        }
        let launch_url = Url::parse(&detail.cloud_pc_launch_url).map_err(|_| Error::InvalidLaunchUrl)?;
        if launch_url.scheme() != "https"
            || launch_url.host_str().is_none()
            || !launch_url.username().is_empty()
            || launch_url.password().is_some()
            || launch_url.port_or_known_default() != Some(443)
        {
            return Err(Error::InvalidLaunchUrl);
        }
        Ok(CloudPcLaunchDetail {
            cloud_pc_id: detail.cloud_pc_id,
            launch_url,
            windows365_switch_compatible: detail.windows365_switch_compatible,
            windows365_switch_compatibility_failure_reason_type: detail
                .windows365_switch_compatibility_failure_reason_type,
        })
    }

    async fn get(&self, url: Url, access_token: &str) -> Result<Vec<u8>, Error> {
        if access_token.is_empty()
            || !access_token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
        {
            return Err(Error::InvalidInput("Graph bearer token"));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| Error::InvalidInput("Graph bearer token"))?;
        authorization.set_sensitive(true);
        let mut response = self
            .http
            .get(url)
            .header(AUTHORIZATION, authorization)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| Error::Transport(error.without_url()))?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|size| usize::try_from(size).map_or(true, |size| MAX_PAGE_BYTES < size))
        {
            return Err(Error::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::Transport(error.without_url()))?
        {
            if MAX_PAGE_BYTES - body.len() < chunk.len() {
                return Err(Error::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn launch_detail_request_url(cloud_pc_id: &str) -> Result<Url, Error> {
    if cloud_pc_id.is_empty()
        || !cloud_pc_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(Error::InvalidInput("Cloud PC ID"));
    }
    let mut url = Url::parse(CLOUD_PCS_URL).map_err(|_| Error::InvalidInput("Cloud PC request URL"))?;
    url.path_segments_mut()
        .map_err(|()| Error::InvalidInput("Cloud PC request URL"))?
        .extend([cloud_pc_id, "retrieveCloudPcLaunchDetail"]);
    Ok(url)
}

fn continuation_url(value: &str) -> Result<Url, Error> {
    let url = Url::parse(value).map_err(|_| Error::InvalidContinuation)?;
    if url.scheme() != "https"
        || url.host_str() != Some("graph.microsoft.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/v1.0/me/cloudPCs"
        || url.fragment().is_some()
    {
        return Err(Error::InvalidContinuation);
    }
    Ok(url)
}

/// Build the [Windows 365 direct-launch URL] for a Cloud PC ID.
///
/// Only provisioned Enterprise and Flex Dedicated Cloud PCs support this browser handoff.
/// The browser handles authentication; no bearer token is placed in the URL.
///
/// [Windows 365 direct-launch URL]: https://learn.microsoft.com/windows-app/direct-launch-urls
pub fn windows365_launch_url(cloud_pc_id: &str, tenant_id: Option<&str>) -> Result<String, Error> {
    launch_url(&["ent", cloud_pc_id], tenant_id)
}

/// Build the [AVD direct-launch URL] from workspace and desktop/RemoteApp object IDs.
///
/// These are object IDs, not Azure Resource Manager paths.
/// The resource must be assigned to the user; external identities also need `tenant_id`.
///
/// [AVD direct-launch URL]: https://learn.microsoft.com/windows-app/direct-launch-urls
pub fn avd_launch_url(workspace_id: &str, resource_id: &str, tenant_id: Option<&str>) -> Result<String, Error> {
    launch_url(&["avd", workspace_id, resource_id], tenant_id)
}

fn launch_url(segments: &[&str], tenant_id: Option<&str>) -> Result<String, Error> {
    if segments
        .iter()
        .chain(tenant_id.iter())
        .any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)))
    {
        return Err(Error::InvalidInput("cloud resource or tenant ID"));
    }
    let mut url =
        Url::parse("https://windows.cloud.microsoft/webclient/").map_err(|_| Error::InvalidInput("Windows App URL"))?;
    url.path_segments_mut()
        .map_err(|()| Error::InvalidInput("Windows App URL"))?
        .pop_if_empty()
        .extend(segments);
    if let Some(tenant_id) = tenant_id {
        url.query_pairs_mut().append_pair("tenant", tenant_id);
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::*;

    const PC_ID: &str = "662009bc-7732-4f6f-8726-25883518ffff";

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
            windows365_launch_url(PC_ID, None).unwrap()
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

    #[test]
    fn launch_detail_uses_documented_graph_resource_route() {
        assert_eq!(
            launch_detail_request_url(PC_ID).unwrap().as_str(),
            format!("{CLOUD_PCS_URL}/{PC_ID}/retrieveCloudPcLaunchDetail")
        );
        for invalid in [
            "",
            "../other",
            "id/path",
            "id?access_token=secret",
            "id#fragment",
            "a b",
        ] {
            assert!(matches!(
                launch_detail_request_url(invalid),
                Err(Error::InvalidInput("Cloud PC ID"))
            ));
        }
    }

    #[test]
    fn continuations_cannot_change_bearer_destination() {
        for invalid in [
            "http://graph.microsoft.com/v1.0/me/cloudPCs",
            "https://attacker.invalid/v1.0/me/cloudPCs",
            "https://graph.microsoft.com.attacker.invalid/v1.0/me/cloudPCs",
            "https://graph.microsoft.com:444/v1.0/me/cloudPCs",
            "https://user:password@graph.microsoft.com/v1.0/me/cloudPCs",
            "https://graph.microsoft.com/v1.0/users",
            "https://graph.microsoft.com/v1.0/me/cloudPCs#fragment",
            "https://graph.microsoft.com/beta/me/cloudPCs",
            "/v1.0/me/cloudPCs?$skiptoken=x",
        ] {
            assert!(matches!(continuation_url(invalid), Err(Error::InvalidContinuation)));
            let body = format!(r#"{{"value":[],"@odata.nextLink":"{invalid}"}}"#);
            assert!(matches!(
                CloudPcPage::decode(body.as_bytes()),
                Err(Error::InvalidContinuation)
            ));
        }
        let next = format!("{CLOUD_PCS_URL}?$skiptoken=opaque%2Bvalue");
        assert_eq!(continuation_url(&next).unwrap().as_str(), next);
    }

    #[test]
    fn invalid_discovery_is_not_an_empty_success() {
        for invalid in [
            &b""[..],
            &b"not JSON"[..],
            &br#"{}"#[..],
            &br#"{"value":null}"#[..],
            &br#"{"value":[{"displayName":"missing ID"}]}"#[..],
            &br#"{"value":[{"id":42}]}"#[..],
        ] {
            assert!(matches!(CloudPcPage::decode(invalid), Err(Error::InvalidResponse(_))));
        }
    }

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

    #[tokio::test]
    async fn launch_detail_rejects_mismatched_or_untrusted_service_urls() {
        let wrong_id = format!(
            r#"{{"cloudPcId":"another-id","cloudPcLaunchUrl":"https://windows.cloud.microsoft/","windows365SwitchCompatible":true}}"#
        );
        let (url, request) = serve(http_response("200 OK", &wrong_id)).await;
        assert!(matches!(
            test_client().fetch_launch_detail(url, "token", PC_ID).await,
            Err(Error::LaunchDetailMismatch)
        ));
        request.await.unwrap();

        for launch_url in [
            "http://rdweb.example/launch",
            "https://user:password@rdweb.example/launch",
            "https://rdweb.example:444/launch",
            "not a URL",
        ] {
            let body = format!(
                r#"{{"cloudPcId":"{PC_ID}","cloudPcLaunchUrl":"{launch_url}","windows365SwitchCompatible":true}}"#
            );
            let (url, request) = serve(http_response("200 OK", &body)).await;
            assert!(matches!(
                test_client().fetch_launch_detail(url, "token", PC_ID).await,
                Err(Error::InvalidLaunchUrl)
            ));
            request.await.unwrap();
        }
    }

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

    #[tokio::test]
    async fn http_status_and_redirects_are_not_success() {
        for status in [
            "401 Unauthorized",
            "403 Forbidden",
            "429 Too Many Requests",
            "503 Unavailable",
        ] {
            let (url, request) = serve(http_response(status, "sensitive service error")).await;
            let error = test_client().fetch_page(url, "test-token").await.unwrap_err();
            assert!(matches!(error, Error::HttpStatus(code) if code.to_string() == status[..3]));
            assert!(!format!("{error:?}").contains("sensitive"));
            request.await.unwrap();
        }
        let (url, request) = serve(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/token-leak\r\nContent-Length: 0\r\n\r\n".to_vec(),
        )
        .await;
        assert!(matches!(
            test_client().fetch_page(url, "test-token").await,
            Err(Error::HttpStatus(302))
        ));
        request.await.unwrap();
    }

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

    #[tokio::test]
    async fn transport_errors_do_not_expose_continuation_urls() {
        let (url, request) = serve(b"not an HTTP response".to_vec()).await;
        let error = test_client().fetch_page(url, "test-token").await.unwrap_err();
        assert!(matches!(&error, Error::Transport(source) if source.url().is_none()));
        assert!(!format!("{error:?}").contains("test-token"));
        request.await.unwrap();
    }

    fn test_client() -> CloudPcClient {
        CloudPcClient {
            http: CloudPcClient::client_builder().no_proxy().build().unwrap(),
        }
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
}
