//! Cloud PC discovery and Windows App browser handoff, not native AVD reverse-connect.
//!
//! Authentication and browser launch belong to the caller.
//! Graph bearer tokens are not RDP session credentials.

use core::fmt;
use core::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use picky::hash::HashAlgorithm;
use picky::key::PublicKey;
use picky::signature::SignatureAlgorithm;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue, USER_AGENT};
use serde::Deserialize;
use url::Url;
use x509_cert::Certificate;
use x509_cert::der::{Decode as _, Encode as _};

const CLOUD_PCS_URL: &str = "https://graph.microsoft.com/v1.0/me/cloudPCs";
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const AVD_FEED_USER_AGENT: &str = "TSWorkspace/2.0";

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    InvalidContinuation,
    TlsConfiguration(std::io::Error),
    Transport(reqwest::Error),
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidResponse(serde_json::Error),
    InvalidXml(&'static str),
    InvalidLaunchUrl,
    LaunchDetailMismatch,
    InvalidRdpSignature(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(field) => write!(f, "invalid {field}"),
            Self::InvalidContinuation => f.write_str("invalid Cloud PC continuation URL"),
            Self::TlsConfiguration(_) => f.write_str("TLS client configuration failed"),
            Self::Transport(_) => f.write_str("Cloud PC service request failed"),
            Self::HttpStatus(status) => write!(f, "Cloud PC service returned HTTP {status}"),
            Self::ResponseTooLarge => f.write_str("Cloud PC response exceeds 1 MiB"),
            Self::InvalidResponse(_) => f.write_str("invalid Cloud PC response"),
            Self::InvalidXml(reason) => write!(f, "invalid AVD feed XML: {reason}"),
            Self::InvalidLaunchUrl => f.write_str("invalid Cloud PC launch URL"),
            Self::LaunchDetailMismatch => f.write_str("Cloud PC launch detail does not match the requested device"),
            Self::InvalidRdpSignature(reason) => write!(f, "invalid .rdp file signature: {reason}"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::TlsConfiguration(error) => Some(error),
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
    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn decode(body: &[u8]) -> Result<Self, Error> {
        Self::decode_page(body)
    }

    fn decode_page(body: &[u8]) -> Result<Self, Error> {
        if MAX_PAGE_BYTES < body.len() {
            return Err(Error::ResponseTooLarge);
        }
        let page: Self = serde_json::from_slice(body).map_err(Error::InvalidResponse)?;
        if let Some(next_link) = &page.next_link {
            continuation_url_inner(next_link)?;
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
        let http = Self::client_builder()?
            .build()
            .map_err(|error| Error::Transport(error.without_url()))?;
        Ok(Self { http })
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub fn new_no_proxy() -> Result<Self, Error> {
        let http = Self::client_builder()?
            .no_proxy()
            .build()
            .map_err(|error| Error::Transport(error.without_url()))?;
        Ok(Self { http })
    }

    fn client_builder() -> Result<reqwest::ClientBuilder, Error> {
        Ok(crate::http_client_builder()
            .map_err(Error::TlsConfiguration)?
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30)))
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
        let url = continuation_url_inner(next_link.unwrap_or(CLOUD_PCS_URL))?;
        self.fetch_page_inner(url, access_token).await
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub async fn fetch_page(&self, url: Url, access_token: &str) -> Result<CloudPcPage, Error> {
        self.fetch_page_inner(url, access_token).await
    }

    async fn fetch_page_inner(&self, url: Url, access_token: &str) -> Result<CloudPcPage, Error> {
        let body = self.get(url, access_token).await?;
        CloudPcPage::decode_page(&body)
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
        let url = launch_detail_request_url_inner(cloud_pc_id)?;
        self.fetch_launch_detail_inner(url, access_token, cloud_pc_id).await
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub async fn fetch_launch_detail(
        &self,
        url: Url,
        access_token: &str,
        expected_cloud_pc_id: &str,
    ) -> Result<CloudPcLaunchDetail, Error> {
        self.fetch_launch_detail_inner(url, access_token, expected_cloud_pc_id)
            .await
    }

    async fn fetch_launch_detail_inner(
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

    /// Discover AVD workspace feed URLs from an RDWeb `api/arm/feeddiscovery` endpoint.
    ///
    /// Supply an Entra bearer token for the AVD RDWeb resource (e.g. `https://www.wvd.microsoft.com`).
    pub async fn discover_avd_feeds(
        &self,
        discovery_url: Url,
        access_token: &str,
    ) -> Result<Vec<AvdFeedSubscription>, Error> {
        let expected_env = validate_avd_service_url(&discovery_url)?;
        let body = self
            .get_with_accept(
                discovery_url,
                access_token,
                "application/x-msts-radc-discovery+xml, application/xml, text/xml",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let xml = core::str::from_utf8(&body).map_err(|_| Error::InvalidXml("response is not valid UTF-8"))?;
        let subscriptions = parse_avd_feed_discovery_xml(xml)?;
        for sub in &subscriptions {
            if validate_avd_service_url(&sub.feed_url)? != expected_env {
                return Err(Error::InvalidXml("FeedURL cloud environment mismatch"));
            }
        }
        Ok(subscriptions)
    }

    /// Fetch and parse an AVD workspace webfeed (`application/x-msts-radc+xml`).
    pub async fn fetch_avd_workspace_feed(&self, feed_url: Url, access_token: &str) -> Result<AvdWorkspaceFeed, Error> {
        let expected_env = validate_avd_service_url(&feed_url)?;
        let body = self
            .get_with_accept(
                feed_url,
                access_token,
                "application/x-msts-radc+xml, application/xml, text/xml",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let xml = core::str::from_utf8(&body).map_err(|_| Error::InvalidXml("response is not valid UTF-8"))?;
        let feed = parse_avd_workspace_feed_xml(xml)?;
        for resource in &feed.resources {
            if let Some(rdp_url) = &resource.rdp_file_url
                && validate_avd_service_url(rdp_url)? != expected_env
            {
                return Err(Error::InvalidXml("ResourceFile URL cloud environment mismatch"));
            }
        }
        Ok(feed)
    }

    /// Download a signed `.rdp` file from an AVD workspace resource URL and verify its signature.
    pub async fn download_avd_rdp_file(
        &self,
        rdp_url: Url,
        access_token: &str,
    ) -> Result<(String, VerifiedRdpSignature), Error> {
        validate_avd_service_url(&rdp_url)?;
        self.download_avd_rdp_file_unchecked_url(rdp_url, access_token, None, None)
            .await
    }

    #[cfg(feature = "__test")]
    #[doc(hidden)]
    pub async fn download_avd_rdp_file_at(
        &self,
        rdp_url: Url,
        access_token: &str,
        trust_anchors_der: Option<&[&[u8]]>,
        validation_time: Option<Duration>,
    ) -> Result<(String, VerifiedRdpSignature), Error> {
        self.download_avd_rdp_file_unchecked_url(rdp_url, access_token, trust_anchors_der, validation_time)
            .await
    }

    async fn download_avd_rdp_file_unchecked_url(
        &self,
        rdp_url: Url,
        access_token: &str,
        trust_anchors_der: Option<&[&[u8]]>,
        validation_time: Option<Duration>,
    ) -> Result<(String, VerifiedRdpSignature), Error> {
        let body = self
            .get_with_accept(
                rdp_url,
                access_token,
                "application/x-rdp, text/plain, */*",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let rdp_text = decode_rdp_bytes(&body)?;
        verify_and_canonicalize_rdp(&rdp_text, trust_anchors_der, validation_time)
    }

    async fn get(&self, url: Url, access_token: &str) -> Result<Vec<u8>, Error> {
        self.get_with_accept(url, access_token, "application/json", None).await
    }

    async fn get_with_accept(
        &self,
        url: Url,
        access_token: &str,
        accept: &'static str,
        user_agent: Option<&'static str>,
    ) -> Result<Vec<u8>, Error> {
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
        let mut req = self
            .http
            .get(url)
            .header(AUTHORIZATION, authorization)
            .header(ACCEPT, accept);
        if let Some(ua) = user_agent {
            req = req.header(USER_AGENT, ua);
        }
        let mut response = req
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

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn launch_detail_request_url(cloud_pc_id: &str) -> Result<Url, Error> {
    launch_detail_request_url_inner(cloud_pc_id)
}

fn launch_detail_request_url_inner(cloud_pc_id: &str) -> Result<Url, Error> {
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

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn continuation_url(value: &str) -> Result<Url, Error> {
    continuation_url_inner(value)
}

fn continuation_url_inner(value: &str) -> Result<Url, Error> {
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

/// Azure Virtual Desktop / Windows 365 cloud sovereign environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvdCloudEnvironment {
    /// Azure Public Cloud (`https://www.wvd.microsoft.com/`).
    Public,
    /// Azure US Government (`https://www.wvd.azure.us/`).
    UsGovernment,
    /// Azure China 21Vianet (`https://www.wvd.azure.cn/`).
    China,
    /// Microsoft Selfhost (`https://www.wvdselfhost.microsoft.com/`).
    Selfhost,
}

impl AvdCloudEnvironment {
    /// Base URL for RDWeb feed discovery in this cloud environment.
    pub fn base_url(self) -> &'static str {
        match self {
            Self::Public => "https://www.wvd.microsoft.com/",
            Self::UsGovernment => "https://www.wvd.azure.us/",
            Self::China => "https://www.wvd.azure.cn/",
            Self::Selfhost => "https://www.wvdselfhost.microsoft.com/",
        }
    }

    /// Build the `api/arm/feeddiscovery` URL for this environment and optional Entra tenant ID.
    pub fn feed_discovery_url(self, tenant_id: Option<&str>) -> Result<Url, Error> {
        if let Some(tid) = tenant_id
            && (tid.is_empty() || !tid.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
        {
            return Err(Error::InvalidInput("cloud resource or tenant ID"));
        }
        let mut url = Url::parse(self.base_url()).map_err(|_| Error::InvalidInput("AVD feed discovery URL"))?;
        url.set_path("api/arm/feeddiscovery");
        if let Some(tid) = tenant_id {
            url.query_pairs_mut().append_pair("aadtenant", tid);
        }
        Ok(url)
    }

    fn from_host(host: &str) -> Option<Self> {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host_matches_domain(host, "wvd.microsoft.com") {
            Some(Self::Public)
        } else if host_matches_domain(host, "wvd.azure.us") {
            Some(Self::UsGovernment)
        } else if host_matches_domain(host, "wvd.azure.cn") {
            Some(Self::China)
        } else if host_matches_domain(host, "wvdselfhost.microsoft.com") {
            Some(Self::Selfhost)
        } else {
            None
        }
    }
}

fn host_matches_domain(host: &str, domain: &str) -> bool {
    if host.eq_ignore_ascii_case(domain) {
        return true;
    }
    if host.len() <= domain.len() + 1 {
        return false;
    }
    let split = host.len() - domain.len();
    host.as_bytes()[split - 1] == b'.' && host.as_bytes()[split..].eq_ignore_ascii_case(domain.as_bytes())
}

/// One tenant/workspace feed entry returned by RDWeb `/api/arm/feeddiscovery`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdFeedSubscription {
    pub tenant_id: String,
    pub tenant_display_name: Option<String>,
    pub feed_url: Url,
    pub hub_discovery_geo_url: Option<Url>,
    pub diagnostic_service_url: Option<Url>,
    pub activity_hint: Option<String>,
    pub conditional_access_claims: Option<String>,
}

/// Kind of published AVD resource in a workspace feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvdResourceKind {
    /// Full session desktop (`Desktop`).
    Desktop,
    /// Published RemoteApp program (`RemoteApp`).
    RemoteApp,
}

/// One published desktop or RemoteApp resource from an AVD workspace webfeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdResource {
    pub id: String,
    pub alias: Option<String>,
    pub title: String,
    pub kind: AvdResourceKind,
    pub arm_path: Option<String>,
    pub device_state: Option<String>,
    pub rdp_file_url: Option<Url>,
    pub rdp_etag: Option<String>,
    pub embedded_rdp: Option<String>,
}

/// Parsed AVD workspace webfeed (`ResourceCollection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvdWorkspaceFeed {
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    pub resources: Vec<AvdResource>,
}

fn validate_avd_service_url(url: &Url) -> Result<AvdCloudEnvironment, Error> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
    {
        return Err(Error::InvalidLaunchUrl);
    }
    let Some(url::Host::Domain(domain)) = url.host() else {
        return Err(Error::InvalidLaunchUrl);
    };
    AvdCloudEnvironment::from_host(domain).ok_or(Error::InvalidLaunchUrl)
}

/// Parse an RDWeb `/api/arm/feeddiscovery` XML document into workspace feed subscriptions.
pub fn parse_avd_feed_discovery_xml(xml: &str) -> Result<Vec<AvdFeedSubscription>, Error> {
    if xml.len() > MAX_PAGE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    let mut subscriptions = Vec::new();
    for tag in find_xml_open_tags(xml)? {
        let Some(feed_url_str) = xml_attr(tag, "FeedURL")? else {
            continue;
        };
        let feed_url = Url::parse(&feed_url_str).map_err(|_| Error::InvalidXml("invalid FeedURL"))?;
        let env = validate_avd_service_url(&feed_url).map_err(|_| Error::InvalidXml("untrusted FeedURL"))?;
        let tenant_id = xml_attr(tag, "TenantId")?.unwrap_or_default();
        if tenant_id.is_empty() {
            return Err(Error::InvalidXml("missing TenantId on FeedURL element"));
        }
        let hub_discovery_geo_url = match xml_attr(tag, "HubDiscoveryGeoURL")? {
            Some(s) if !s.is_empty() => {
                let u = Url::parse(&s).map_err(|_| Error::InvalidXml("invalid HubDiscoveryGeoURL"))?;
                let hub_env =
                    validate_avd_service_url(&u).map_err(|_| Error::InvalidXml("untrusted HubDiscoveryGeoURL"))?;
                if hub_env != env {
                    return Err(Error::InvalidXml("HubDiscoveryGeoURL cloud environment mismatch"));
                }
                Some(u)
            }
            _ => None,
        };
        let diagnostic_service_url = match xml_attr(tag, "DiagnosticServiceURL")? {
            Some(s) if !s.is_empty() => {
                let u = Url::parse(&s).map_err(|_| Error::InvalidXml("invalid DiagnosticServiceURL"))?;
                let diag_env =
                    validate_avd_service_url(&u).map_err(|_| Error::InvalidXml("untrusted DiagnosticServiceURL"))?;
                if diag_env != env {
                    return Err(Error::InvalidXml("DiagnosticServiceURL cloud environment mismatch"));
                }
                Some(u)
            }
            _ => None,
        };
        subscriptions.push(AvdFeedSubscription {
            tenant_id,
            tenant_display_name: xml_attr(tag, "TenantDisplayName")?,
            feed_url,
            hub_discovery_geo_url,
            diagnostic_service_url,
            activity_hint: xml_attr(tag, "ActivityHint")?,
            conditional_access_claims: xml_attr(tag, "ConditionalAccessClaims")?,
        });
    }
    if subscriptions.is_empty() {
        return Err(Error::InvalidXml("no FeedURL entries found"));
    }
    Ok(subscriptions)
}

/// Parse an RDWeb `application/x-msts-radc+xml` workspace webfeed XML document.
pub fn parse_avd_workspace_feed_xml(xml: &str) -> Result<AvdWorkspaceFeed, Error> {
    if xml.len() > MAX_PAGE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    let top_tags = find_xml_open_tags(xml)?;
    if !top_tags.iter().any(|tag| {
        let name = tag_name(tag);
        name.eq_ignore_ascii_case("ResourceCollection") || name.eq_ignore_ascii_case("Publisher")
    }) {
        return Err(Error::InvalidXml("missing ResourceCollection or Publisher element"));
    }

    let mut workspace_id = None;
    let mut workspace_name = None;
    for tag in &top_tags {
        if tag_name(tag).eq_ignore_ascii_case("Publisher") {
            workspace_id = xml_attr(tag, "ID")?;
            workspace_name = xml_attr(tag, "Name")?;
            break;
        }
    }

    let mut resources = Vec::new();
    let mut search_rest = xml;
    while let Some((res_start, open_end)) = find_open_tag_span(search_rest, "Resource")? {
        let after_start = search_rest
            .get(res_start..)
            .ok_or(Error::InvalidXml("invalid Resource span"))?;
        let open_tag = after_start
            .get(..=open_end)
            .ok_or(Error::InvalidXml("invalid Resource tag"))?;
        let (inner, next_offset) = if open_tag.ends_with("/>") {
            ("", res_start + open_end + 1)
        } else {
            let body_start = open_end + 1;
            let body_slice = after_start
                .get(body_start..)
                .ok_or(Error::InvalidXml("invalid Resource body"))?;
            let (close_rel_start, close_rel_end) = find_close_tag_span(body_slice, "Resource")?
                .ok_or(Error::InvalidXml("missing closing </Resource> tag"))?;
            let inner_slice = body_slice
                .get(..close_rel_start)
                .ok_or(Error::InvalidXml("invalid Resource inner span"))?;
            (inner_slice, res_start + body_start + close_rel_end)
        };
        search_rest = search_rest
            .get(next_offset..)
            .ok_or(Error::InvalidXml("invalid Resource continuation"))?;

        let id = xml_attr(open_tag, "ID")?.ok_or(Error::InvalidXml("Resource missing ID attribute"))?;
        let title = xml_attr(open_tag, "Title")?.unwrap_or_else(|| id.clone());
        let alias = xml_attr(open_tag, "Alias")?;
        let kind = match xml_attr(open_tag, "Type")?.as_deref() {
            Some(t) if t.eq_ignore_ascii_case("RemoteApp") => AvdResourceKind::RemoteApp,
            _ => AvdResourceKind::Desktop,
        };
        let mut arm_path = xml_attr(open_tag, "ArmPath")?;
        let mut device_state = xml_attr(open_tag, "DeviceState")?;
        let mut rdp_file_url = None;
        let mut rdp_etag = None;

        for child_tag in find_xml_open_tags(inner)? {
            let name = tag_name(child_tag);
            if arm_path.is_none() {
                arm_path = xml_attr(child_tag, "ArmPath")?;
            }
            if device_state.is_none() {
                device_state = xml_attr(child_tag, "DeviceState")?;
            }
            if name.eq_ignore_ascii_case("ResourceFile") {
                let ext = xml_attr(child_tag, "FileExtension")?.unwrap_or_default();
                if ext.is_empty() || ext.eq_ignore_ascii_case(".rdp") || ext.eq_ignore_ascii_case("rdp") {
                    if let Some(url_str) = xml_attr(child_tag, "URL")? {
                        let parsed = Url::parse(&url_str).map_err(|_| Error::InvalidXml("invalid ResourceFile URL"))?;
                        validate_avd_service_url(&parsed)
                            .map_err(|_| Error::InvalidXml("untrusted ResourceFile URL"))?;
                        rdp_file_url = Some(parsed);
                    }
                    rdp_etag = xml_attr(child_tag, "ETag")?;
                }
            }
        }

        let embedded_rdp = match extract_xml_element_text(inner, "RDPFile")? {
            Some(raw) if !raw.trim().is_empty() => {
                let unescaped = unescape_xml(raw)?;
                let (canonical_rdp, _) = verify_and_canonicalize_rdp(&unescaped, None, None)?;
                Some(canonical_rdp)
            }
            _ => None,
        };

        resources.push(AvdResource {
            id,
            alias,
            title,
            kind,
            arm_path,
            device_state,
            rdp_file_url,
            rdp_etag,
            embedded_rdp,
        });
    }

    Ok(AvdWorkspaceFeed {
        workspace_id,
        workspace_name,
        resources,
    })
}

fn find_tag_end(tag_from_lt: &str) -> Result<usize, Error> {
    let bytes = tag_from_lt.as_bytes();
    let mut in_quote: Option<u8> = None;
    for (idx, &b) in bytes.iter().enumerate().skip(1) {
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => in_quote = Some(b),
                b'>' => return Ok(idx),
                b'<' => return Err(Error::InvalidXml("unexpected '<' inside XML tag")),
                _ => {}
            }
        }
    }
    Err(Error::InvalidXml("unclosed XML tag"))
}

fn skip_xml_special(after_lt: &str) -> Result<Option<usize>, Error> {
    if let Some(comment) = after_lt.strip_prefix("<!--") {
        let end = comment.find("-->").ok_or(Error::InvalidXml("unclosed XML comment"))?;
        Ok(Some(4 + end + 3))
    } else if let Some(cdata) = after_lt.strip_prefix("<![CDATA[") {
        let end = cdata.find("]]>").ok_or(Error::InvalidXml("unclosed CDATA section"))?;
        Ok(Some(9 + end + 3))
    } else if let Some(pi) = after_lt.strip_prefix("<?") {
        let end = pi
            .find("?>")
            .ok_or(Error::InvalidXml("unclosed XML processing instruction"))?;
        Ok(Some(2 + end + 2))
    } else if after_lt.starts_with("<!") {
        let end = find_tag_end(after_lt)?;
        Ok(Some(end + 1))
    } else {
        Ok(None)
    }
}

fn matches_tag_name_at(after_name_start: &[u8], name: &str) -> bool {
    let name_bytes = name.as_bytes();
    after_name_start.len() > name_bytes.len()
        && after_name_start[..name_bytes.len()].eq_ignore_ascii_case(name_bytes)
        && matches!(
            after_name_start[name_bytes.len()],
            b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/'
        )
}

fn find_open_tag_span(haystack: &str, name: &str) -> Result<Option<(usize, usize)>, Error> {
    let mut offset = 0;
    while let Some(tail) = haystack.get(offset..) {
        let Some(pos) = tail.find('<') else {
            break;
        };
        let idx = offset + pos;
        let after = haystack.get(idx..).ok_or(Error::InvalidXml("invalid XML offset"))?;
        if let Some(skip_len) = skip_xml_special(after)? {
            offset = idx + skip_len;
            continue;
        }
        let end = find_tag_end(after)?;
        if !after.starts_with("</") && matches_tag_name_at(&after.as_bytes()[1..], name) {
            return Ok(Some((idx, end)));
        }
        offset = idx + end + 1;
    }
    Ok(None)
}

fn find_close_tag_span(haystack: &str, name: &str) -> Result<Option<(usize, usize)>, Error> {
    let mut offset = 0;
    while let Some(tail) = haystack.get(offset..) {
        let Some(pos) = tail.find('<') else {
            break;
        };
        let idx = offset + pos;
        let after = haystack.get(idx..).ok_or(Error::InvalidXml("invalid XML offset"))?;
        if let Some(skip_len) = skip_xml_special(after)? {
            offset = idx + skip_len;
            continue;
        }
        let end = find_tag_end(after)?;
        if after.starts_with("</") && matches_tag_name_at(&after.as_bytes()[2..], name) {
            return Ok(Some((idx, idx + end + 1)));
        }
        offset = idx + end + 1;
    }
    Ok(None)
}

fn find_xml_open_tags(xml: &str) -> Result<Vec<&str>, Error> {
    let mut tags = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let after = rest.get(start..).ok_or(Error::InvalidXml("invalid XML offset"))?;
        if let Some(skip_len) = skip_xml_special(after)? {
            rest = after
                .get(skip_len..)
                .ok_or(Error::InvalidXml("invalid XML comment or CDATA span"))?;
            continue;
        }
        let end = find_tag_end(after)?;
        let tag = after.get(..=end).ok_or(Error::InvalidXml("invalid XML tag span"))?;
        if !tag.starts_with("</") {
            tags.push(tag);
        }
        rest = after
            .get(end + 1..)
            .ok_or(Error::InvalidXml("invalid XML continuation"))?;
    }
    Ok(tags)
}

fn tag_name(tag: &str) -> &str {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    inner.split_whitespace().next().unwrap_or("")
}

fn xml_attr(tag: &str, attr_name: &str) -> Result<Option<String>, Error> {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    let Some((_, after_name)) = inner.split_once(|c: char| c.is_whitespace()) else {
        return Ok(None);
    };
    let mut rest = after_name.trim_start();
    while !rest.is_empty() {
        let Some((before_eq, after_eq)) = rest.split_once('=') else {
            break;
        };
        let key = before_eq.trim();
        let after = after_eq.trim_start();
        let Some(quote) = after.chars().next() else {
            return Err(Error::InvalidXml("missing attribute value"));
        };
        if quote != '"' && quote != '\'' {
            return Err(Error::InvalidXml("unquoted XML attribute value"));
        }
        let value_rest = after
            .strip_prefix(quote)
            .ok_or(Error::InvalidXml("missing attribute opening quote"))?;
        let Some((raw_val, tail)) = value_rest.split_once(quote) else {
            return Err(Error::InvalidXml("unclosed XML attribute quote"));
        };
        if key.eq_ignore_ascii_case(attr_name) {
            return Ok(Some(unescape_xml(raw_val)?));
        }
        rest = tail.trim_start();
    }
    Ok(None)
}

fn extract_xml_element_text<'a>(xml: &'a str, element_name: &str) -> Result<Option<&'a str>, Error> {
    let Some((start_idx, open_end)) = find_open_tag_span(xml, element_name)? else {
        return Ok(None);
    };
    let after = xml.get(start_idx..).ok_or(Error::InvalidXml("invalid element start"))?;
    let open_tag = after
        .get(..=open_end)
        .ok_or(Error::InvalidXml("invalid element open tag"))?;
    if open_tag.ends_with("/>") {
        return Ok(None);
    }
    let body = after
        .get(open_end + 1..)
        .ok_or(Error::InvalidXml("invalid element body"))?;
    let (close_pos, _) =
        find_close_tag_span(body, element_name)?.ok_or(Error::InvalidXml("missing closing element tag"))?;
    body.get(..close_pos)
        .map(Some)
        .ok_or(Error::InvalidXml("invalid element body span"))
}

fn unescape_xml(input: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some((before_amp, after_amp)) = rest.split_once('&') {
        out.push_str(before_amp);
        let Some((entity, after_semi)) = after_amp.split_once(';') else {
            return Err(Error::InvalidXml("unterminated XML entity"));
        };
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).map_err(|_| Error::InvalidXml("invalid XML hex entity"))?
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>()
                        .map_err(|_| Error::InvalidXml("invalid XML numeric entity"))?
                } else {
                    return Err(Error::InvalidXml("unknown XML entity"));
                };
                let ch = char::from_u32(code).ok_or(Error::InvalidXml("invalid XML character reference"))?;
                out.push(ch);
            }
        }
        rest = after_semi;
    }
    out.push_str(rest);
    Ok(out)
}

/// Sensitive `.rdp` settings that must be covered by `signscope` whenever present in a signed `.rdp` file,
/// matching the 48 `SENSITIVE_SETTINGS` in `mstscax.dll` (`ironrdp-activex`) plus AVD and IronRDP client properties.
const SENSITIVE_RDP_SETTINGS: &[&str] = &[
    "full address",
    "alternate full address",
    "pcb",
    "use redirection server name",
    "server port",
    "negotiate security layer",
    "enablecredsspsupport",
    "disableconnectionsharing",
    "autoreconnection enabled",
    "gatewayhostname",
    "gatewayusagemethod",
    "gatewayprofileusagemethod",
    "gatewaycredentialssource",
    "support url",
    "promptcredentialonce",
    "require pre-authentication",
    "pre-authentication server address",
    "alternate shell",
    "shell working directory",
    "remoteapplicationprogram",
    "remoteapplicationexpandworkingdir",
    "remoteapplicationmode",
    "remoteapplicationguid",
    "remoteapplicationname",
    "remoteapplicationicon",
    "remoteapplicationfile",
    "remoteapplicationfileextensions",
    "remoteapplicationcmdline",
    "remoteapplicationexpandcmdline",
    "prompt for credentials",
    "authentication level",
    "audiomode",
    "redirectdrives",
    "redirectprinters",
    "redirectcomports",
    "redirectsmartcards",
    "redirectposdevices",
    "redirectclipboard",
    "devicestoredirect",
    "drivestoredirect",
    "loadbalanceinfo",
    "redirectdirectx",
    "rdgiskdcproxy",
    "kdcproxyname",
    "enablerdsaadauth",
    "redirectwebauthn",
    "redirecttextprocessing",
    "allowed security protocols",
    "armpath",
    "aadtenantid",
    "gatewaybrokeringtype",
    "wvd endpoint pool",
    "workspace id",
    "administrative session",
    "audiocapturemode",
    "camerastoredirect",
    "cleartextpassword",
    "gatewaypassword",
    "gatewayusername",
    "ironrdp_autologon",
    "ironrdp_dvcpipeproxy",
    "ironrdp_dvcplugin",
    "ironrdp_gatewayrpch",
    "ironrdp_named_pipe",
    "ironrdp_rdcleanpathtoken",
    "ironrdp_rdcleanpathurl",
    "ironrdp_rdpdr",
    "ironrdp_smartcard",
    "ironrdp_tls",
    "ironrdp_vmconnect",
    "ironrdp_vmconnect_basic",
    "ironrdp_vmconnect_current_user",
    "kdcproxyurl",
    "redirectlocation",
];

const MAX_SIGN_SCOPE_CHARS: usize = 0x800;
const SIGNATURE_HEADER_LEN: usize = 12;
const SIGNATURE_VERSION_1: u16 = 1;
const SIGNATURE_TYPE_CERTIFICATE: u16 = 1;

// OID 1.2.840.113549.1.7.1 (id-data)
const OID_ID_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01];
// OID 1.2.840.113549.1.7.2 (signedData)
const OID_SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
// OID 1.2.840.113549.1.9.3 (contentType)
const OID_CONTENT_TYPE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03];
// OID 1.2.840.113549.1.9.4 (messageDigest)
const OID_MESSAGE_DIGEST: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04];
// OID 1.2.840.113549.1.1.1 (rsaEncryption)
const OID_RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
// OID 1.2.840.113549.1.1.11 (sha256WithRSAEncryption)
const OID_SHA256_WITH_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
// OID 1.2.840.113549.1.1.12 (sha384WithRSAEncryption)
const OID_SHA384_WITH_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c];
// OID 1.2.840.113549.1.1.13 (sha512WithRSAEncryption)
const OID_SHA512_WITH_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d];

// X.509 extension OIDs
// 2.5.29.14 (id-ce-subjectKeyIdentifier)
const OID_EXT_SKI: &[u8] = &[0x55, 0x1d, 0x0e];
// 2.5.29.15 (id-ce-keyUsage)
const OID_EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
// 2.5.29.17 (id-ce-subjectAltName)
const OID_EXT_SAN: &[u8] = &[0x55, 0x1d, 0x11];
// 2.5.29.19 (id-ce-basicConstraints)
const OID_EXT_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
// 2.5.29.32 (id-ce-certificatePolicies)
const OID_EXT_CERT_POLICIES: &[u8] = &[0x55, 0x1d, 0x20];
// 2.5.29.37 (id-ce-extKeyUsage)
const OID_EXT_EKU: &[u8] = &[0x55, 0x1d, 0x25];

// Allowed RDP signing Extended Key Usage OIDs (matching mstscax.dll):
// 1.3.6.1.5.5.7.3.1 (id-kp-serverAuth)
const OID_EKU_SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
// 1.3.6.1.5.5.7.3.3 (id-kp-codeSigning)
const OID_EKU_CODE_SIGNING: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03];
// 1.3.6.1.4.1.311.54.1.1 (szOID_KP_TS_SIGNING / Microsoft RDP signing)
const OID_EKU_MS_RDP_SIGNING: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x36, 0x01, 0x01];
// 2.5.29.37.0 (anyExtendedKeyUsage)
const OID_EKU_ANY: &[u8] = &[0x55, 0x1d, 0x25, 0x00];

/// Result of verifying the PKCS#7 / CMS signature on a signed `.rdp` file (`signscope` + `signature`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRdpSignature {
    /// Property names covered by `signscope`, in signed order.
    pub signed_scope: Vec<String>,
    /// Subject distinguished name of the signing certificate.
    pub signer_subject: String,
    /// DER-encoded X.509 leaf certificate that signed the `.rdp` file.
    pub signer_certificate_der: Vec<u8>,
}

/// Verify the `signscope` and detached PKCS#7 `signature` of an `.rdp` file against platform root certificates.
///
/// Enforces sensitive-setting coverage (`VerifySensitiveSettings` in `RDCoreSdk.dll`), verifies
/// the detached PKCS#7 RSA signature over the canonical UTF-16LE `signscope` lines, and validates
/// the X.509 certificate chain, validity window, CA constraints, and RDP signing EKU against the
/// platform trust store.
pub fn verify_rdp_signature(rdp_content: &str) -> Result<VerifiedRdpSignature, Error> {
    let (_, verified) = verify_and_canonicalize_rdp(rdp_content, None, None)?;
    Ok(verified)
}

/// Verify the `signscope` and detached PKCS#7 `signature` of an `.rdp` file against explicit DER trust anchors.
pub fn verify_rdp_signature_with_roots(
    rdp_content: &str,
    trust_anchors_der: &[&[u8]],
) -> Result<VerifiedRdpSignature, Error> {
    let (_, verified) = verify_and_canonicalize_rdp(rdp_content, Some(trust_anchors_der), None)?;
    Ok(verified)
}

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn verify_rdp_signature_at(
    rdp_content: &str,
    trust_anchors_der: &[&[u8]],
    validation_time: Duration,
) -> Result<VerifiedRdpSignature, Error> {
    let (_, verified) = verify_and_canonicalize_rdp(rdp_content, Some(trust_anchors_der), Some(validation_time))?;
    Ok(verified)
}

fn decode_rdp_bytes(body: &[u8]) -> Result<String, Error> {
    if let Some(payload) = body.strip_prefix(&[0xFF, 0xFE]) {
        if !payload.len().is_multiple_of(2) {
            return Err(Error::InvalidRdpSignature("RDP file has odd UTF-16LE byte length"));
        }
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let text =
            String::from_utf16(&units).map_err(|_| Error::InvalidRdpSignature("RDP file is not valid UTF-16LE"))?;
        Ok(text.trim_end_matches('\0').to_owned())
    } else if let Some(payload) = body.strip_prefix(&[0xFE, 0xFF]) {
        if !payload.len().is_multiple_of(2) {
            return Err(Error::InvalidRdpSignature("RDP file has odd UTF-16BE byte length"));
        }
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        let text =
            String::from_utf16(&units).map_err(|_| Error::InvalidRdpSignature("RDP file is not valid UTF-16BE"))?;
        Ok(text.trim_end_matches('\0').to_owned())
    } else {
        let text =
            String::from_utf8(body.to_vec()).map_err(|_| Error::InvalidRdpSignature("RDP file is not valid UTF-8"))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        Ok(text.trim_end_matches('\0').to_owned())
    }
}

fn verify_and_canonicalize_rdp(
    rdp_content: &str,
    explicit_trust_anchors: Option<&[&[u8]]>,
    validation_time: Option<Duration>,
) -> Result<(String, VerifiedRdpSignature), Error> {
    let validation_time = match validation_time {
        Some(t) => t,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::InvalidRdpSignature("system clock is before UNIX_EPOCH"))?,
    };

    let content = rdp_content
        .strip_prefix('\u{feff}')
        .unwrap_or(rdp_content)
        .trim_end_matches('\0');
    let mut records_by_key: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut values_by_key: std::collections::HashMap<String, (&str, String)> = std::collections::HashMap::new();

    for raw_line in content.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.trim().is_empty() {
            continue;
        }
        let Some((raw_key, rest)) = line.split_once(':') else {
            return Err(Error::InvalidRdpSignature("malformed .rdp property line"));
        };
        let Some((raw_ty, raw_val)) = rest.split_once(':') else {
            return Err(Error::InvalidRdpSignature("malformed .rdp property line"));
        };
        let key_lower = raw_key.trim().to_ascii_lowercase();
        if key_lower.is_empty() {
            return Err(Error::InvalidRdpSignature("empty .rdp property name"));
        }
        let (ty, canonical_val) = match raw_ty.trim() {
            "s" => ("s", raw_val.to_owned()),
            "i" => {
                let parsed = raw_val
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| Error::InvalidRdpSignature("invalid integer .rdp property"))?;
                ("i", parsed.to_string())
            }
            _ => return Err(Error::InvalidRdpSignature("unsupported .rdp property type")),
        };
        let canonical_line = format!("{key_lower}:{ty}:{canonical_val}");
        if records_by_key.insert(key_lower.clone(), canonical_line).is_some() {
            return Err(Error::InvalidRdpSignature("duplicate .rdp property"));
        }
        values_by_key.insert(key_lower, (ty, canonical_val));
    }

    let (signscope_ty, signscope_value) = values_by_key
        .get("signscope")
        .ok_or(Error::InvalidRdpSignature("missing signscope property"))?;
    if *signscope_ty != "s"
        || signscope_value.is_empty()
        || MAX_SIGN_SCOPE_CHARS <= signscope_value.encode_utf16().count()
    {
        return Err(Error::InvalidRdpSignature("malformed signscope"));
    }

    let mut signed_scope: Vec<String> = Vec::new();
    for raw_token in signscope_value.split(',') {
        if raw_token.is_empty() || raw_token.trim() != raw_token {
            return Err(Error::InvalidRdpSignature("malformed signscope"));
        }
        if raw_token.eq_ignore_ascii_case("signscope") || raw_token.eq_ignore_ascii_case("signature") {
            return Err(Error::InvalidRdpSignature("malformed signscope"));
        }
        if signed_scope
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(raw_token))
        {
            return Err(Error::InvalidRdpSignature("duplicate property in signscope"));
        }
        signed_scope.push(raw_token.to_owned());
    }
    if signed_scope.is_empty() {
        return Err(Error::InvalidRdpSignature("signscope is empty"));
    }

    if !records_by_key.contains_key("full address") {
        return Err(Error::InvalidRdpSignature("missing full address property"));
    }

    for sensitive in SENSITIVE_RDP_SETTINGS {
        if records_by_key.contains_key(*sensitive) && !signed_scope.iter().any(|s| s.eq_ignore_ascii_case(sensitive)) {
            return Err(Error::InvalidRdpSignature(
                "sensitive .rdp setting is not covered by signscope",
            ));
        }
    }

    let mut canonical = String::new();
    for scope_key in &signed_scope {
        let line = records_by_key
            .get(&scope_key.to_ascii_lowercase())
            .ok_or(Error::InvalidRdpSignature("property listed in signscope is missing"))?;
        canonical.push_str(line);
        canonical.push_str("\r\n");
    }
    let signscope_line = records_by_key
        .get("signscope")
        .ok_or(Error::InvalidRdpSignature("missing signscope property"))?;
    canonical.push_str(signscope_line);
    canonical.push_str("\r\n");

    let mut canonical_utf16le: Vec<u8> = Vec::with_capacity((canonical.len() + 1) * 2);
    for unit in canonical.encode_utf16() {
        canonical_utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    canonical_utf16le.extend_from_slice(&[0, 0]);

    let (sig_ty, sig_raw_val) = values_by_key
        .get("signature")
        .ok_or(Error::InvalidRdpSignature("missing signature property"))?;
    if *sig_ty != "s" {
        return Err(Error::InvalidRdpSignature("malformed signature property"));
    }
    let sig_b64: String = sig_raw_val.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let raw_sig = STANDARD
        .decode(&sig_b64)
        .map_err(|_| Error::InvalidRdpSignature("invalid base64 in signature"))?;
    if raw_sig.len() < SIGNATURE_HEADER_LEN {
        return Err(Error::InvalidRdpSignature("signature envelope is truncated"));
    }
    let version = u16::from_le_bytes([raw_sig[0], raw_sig[1]]);
    let sig_type = u16::from_le_bytes([raw_sig[2], raw_sig[3]]);
    let flags = u32::from_le_bytes([raw_sig[4], raw_sig[5], raw_sig[6], raw_sig[7]]);
    let declared_len = usize::try_from(u32::from_le_bytes([raw_sig[8], raw_sig[9], raw_sig[10], raw_sig[11]]))
        .map_err(|_| Error::InvalidRdpSignature("invalid signature length"))?;
    if version != SIGNATURE_VERSION_1 || sig_type != SIGNATURE_TYPE_CERTIFICATE || (flags & 1) == 0 || declared_len == 0
    {
        return Err(Error::InvalidRdpSignature("invalid signature header"));
    }
    let pkcs7_der = &raw_sig[SIGNATURE_HEADER_LEN..];
    if pkcs7_der.len() < declared_len {
        return Err(Error::InvalidRdpSignature(
            "PKCS#7 payload is shorter than header length",
        ));
    }
    let pkcs7_der = &pkcs7_der[..declared_len];

    let parsed_pkcs7 = parse_pkcs7_signed_data(pkcs7_der)?;
    let hash_alg = match parsed_pkcs7.digest_oid {
        // 2.16.840.1.101.3.4.2.1 (id-sha256)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01] => HashAlgorithm::SHA2_256,
        // 2.16.840.1.101.3.4.2.2 (id-sha384)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02] => HashAlgorithm::SHA2_384,
        // 2.16.840.1.101.3.4.2.3 (id-sha512)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03] => HashAlgorithm::SHA2_512,
        _ => return Err(Error::InvalidRdpSignature("unsupported PKCS#7 digest algorithm")),
    };

    match parsed_pkcs7.sig_alg_oid {
        OID_RSA_ENCRYPTION => {}
        OID_SHA256_WITH_RSA if hash_alg == HashAlgorithm::SHA2_256 => {}
        OID_SHA384_WITH_RSA if hash_alg == HashAlgorithm::SHA2_384 => {}
        OID_SHA512_WITH_RSA if hash_alg == HashAlgorithm::SHA2_512 => {}
        _ => return Err(Error::InvalidRdpSignature("unsupported PKCS#7 signature algorithm")),
    }

    let signed_payload = if let Some(attrs_body) = parsed_pkcs7.signed_attrs_raw {
        let expected_digest = hash_alg.digest(&canonical_utf16le);
        verify_signed_attrs(attrs_body, &expected_digest)?;
        encode_der_set(attrs_body)
    } else {
        canonical_utf16le
    };

    let mut parsed_certs: Vec<(&[u8], Certificate)> = Vec::with_capacity(parsed_pkcs7.certs_der.len());
    for &der in &parsed_pkcs7.certs_der {
        let cert = Certificate::from_der(der)
            .map_err(|_| Error::InvalidRdpSignature("invalid X.509 certificate in PKCS#7"))?;
        parsed_certs.push((der, cert));
    }
    if parsed_certs.is_empty() {
        return Err(Error::InvalidRdpSignature("no valid X.509 certificates in PKCS#7"));
    }

    let (leaf_der, leaf_cert) = parsed_certs
        .iter()
        .find(|(_, cert)| matches_signer_id(cert, &parsed_pkcs7.signer_id))
        .ok_or(Error::InvalidRdpSignature(
            "no certificate in PKCS#7 matches SignerInfo sid",
        ))?;

    let spki_der = leaf_cert
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|_| Error::InvalidRdpSignature("failed to encode signer SPKI"))?;
    let public_key =
        PublicKey::from_der(&spki_der).map_err(|_| Error::InvalidRdpSignature("unsupported signer public key"))?;
    let sig_algo = SignatureAlgorithm::RsaPkcs1v15(hash_alg);
    if sig_algo
        .verify(&public_key, &signed_payload, parsed_pkcs7.signature_bytes)
        .is_err()
    {
        return Err(Error::InvalidRdpSignature("RSA signature verification failed"));
    }

    let native_roots;
    let mut parsed_anchors: Vec<Certificate> = Vec::new();
    if let Some(explicit) = explicit_trust_anchors {
        for &der in explicit {
            let cert =
                Certificate::from_der(der).map_err(|_| Error::InvalidRdpSignature("invalid trust anchor DER"))?;
            parsed_anchors.push(cert);
        }
    } else {
        native_roots = rustls_native_certs::load_native_certs();
        for cert_der in &native_roots.certs {
            if let Ok(cert) = Certificate::from_der(cert_der.as_ref()) {
                parsed_anchors.push(cert);
            }
        }
    }
    if parsed_anchors.is_empty() {
        return Err(Error::InvalidRdpSignature("no trusted root certificates available"));
    }

    verify_certificate_chain(leaf_der, leaf_cert, &parsed_certs, &parsed_anchors, validation_time)?;

    let mut normalized_doc = canonical;
    if let Some(sig_line) = records_by_key.get("signature") {
        normalized_doc.push_str(sig_line);
        normalized_doc.push_str("\r\n");
    }

    let signer_subject = leaf_cert.tbs_certificate().subject().to_string();
    Ok((
        normalized_doc,
        VerifiedRdpSignature {
            signed_scope,
            signer_subject,
            signer_certificate_der: (*leaf_der).to_vec(),
        },
    ))
}

enum SignerId<'a> {
    IssuerAndSerialNumber {
        issuer_der: &'a [u8],
        serial_bytes: &'a [u8],
    },
    SubjectKeyIdentifier(&'a [u8]),
}

struct ParsedPkcs7<'a> {
    certs_der: Vec<&'a [u8]>,
    signer_id: SignerId<'a>,
    digest_oid: &'a [u8],
    signed_attrs_raw: Option<&'a [u8]>,
    sig_alg_oid: &'a [u8],
    signature_bytes: &'a [u8],
}

fn matches_signer_id(cert: &Certificate, signer_id: &SignerId<'_>) -> bool {
    match signer_id {
        SignerId::IssuerAndSerialNumber {
            issuer_der,
            serial_bytes,
        } => {
            let Ok(cert_issuer_der) = cert.tbs_certificate().issuer().to_der() else {
                return false;
            };
            let cert_serial = trim_leading_zero_byte(cert.tbs_certificate().serial_number().as_bytes());
            let expected_serial = trim_leading_zero_byte(serial_bytes);
            cert_issuer_der.as_slice() == *issuer_der && cert_serial == expected_serial
        }
        SignerId::SubjectKeyIdentifier(expected_ski) => {
            let Some(exts) = cert.tbs_certificate().extensions() else {
                return false;
            };
            exts.iter().any(|ext| {
                ext.extn_id.as_bytes() == OID_EXT_SKI
                    && read_der_tlv(ext.extn_value.as_bytes())
                        .is_some_and(|(tag, val, rest)| tag == 0x04 && rest.is_empty() && val == *expected_ski)
            })
        }
    }
}

fn trim_leading_zero_byte(bytes: &[u8]) -> &[u8] {
    if bytes.len() > 1 && bytes[0] == 0 {
        &bytes[1..]
    } else {
        bytes
    }
}

#[cfg(feature = "__test")]
#[doc(hidden)]
pub fn verify_signed_attrs_for_test(raw_attributes: &[u8], expected_digest: &[u8]) -> Result<(), Error> {
    verify_signed_attrs(raw_attributes, expected_digest)
}

fn verify_signed_attrs(raw_attributes: &[u8], expected_digest: &[u8]) -> Result<(), Error> {
    let mut rest = raw_attributes;
    let mut message_digest_count = 0usize;
    let mut content_type_count = 0usize;

    while !rest.is_empty() {
        let (item_tag, item_body, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed PKCS#7 signed attribute"))?;
        if item_tag != 0x30 {
            return Err(Error::InvalidRdpSignature("PKCS#7 signed attribute is not a SEQUENCE"));
        }
        let (oid_tag, oid_bytes, item_tail) =
            read_der_tlv(item_body).ok_or(Error::InvalidRdpSignature("missing signed attribute OID"))?;
        if oid_tag != 0x06 {
            return Err(Error::InvalidRdpSignature("invalid signed attribute OID"));
        }
        let (set_tag, set_body, set_rem) =
            read_der_tlv(item_tail).ok_or(Error::InvalidRdpSignature("missing signed attribute values SET"))?;
        if set_tag != 0x31 || !set_rem.is_empty() {
            return Err(Error::InvalidRdpSignature("invalid signed attribute values SET"));
        }
        if oid_bytes == OID_MESSAGE_DIGEST {
            message_digest_count += 1;
            let (val_tag, val_bytes, val_rem) =
                read_der_tlv(set_body).ok_or(Error::InvalidRdpSignature("empty messageDigest attribute"))?;
            if val_tag != 0x04 || !val_rem.is_empty() {
                return Err(Error::InvalidRdpSignature(
                    "messageDigest attribute must be one OCTET STRING",
                ));
            }
            if val_bytes != expected_digest {
                return Err(Error::InvalidRdpSignature("PKCS#7 messageDigest attribute mismatch"));
            }
        } else if oid_bytes == OID_CONTENT_TYPE {
            content_type_count += 1;
            let (val_tag, val_bytes, val_rem) =
                read_der_tlv(set_body).ok_or(Error::InvalidRdpSignature("empty contentType attribute"))?;
            if val_tag != 0x06 || !val_rem.is_empty() || val_bytes != OID_ID_DATA {
                return Err(Error::InvalidRdpSignature("invalid PKCS#7 contentType attribute"));
            }
        }
        rest = next_rest;
    }

    if message_digest_count != 1 || content_type_count > 1 {
        return Err(Error::InvalidRdpSignature(
            "PKCS#7 signedAttrs must contain exactly one messageDigest",
        ));
    }
    Ok(())
}

fn verify_certificate_chain(
    leaf_der: &[u8],
    leaf: &Certificate,
    bag_certs: &[(&[u8], Certificate)],
    trust_anchors: &[Certificate],
    validation_time: Duration,
) -> Result<(), Error> {
    check_cert_validity(leaf, validation_time)?;
    check_leaf_extensions(leaf)?;

    let mut current = leaf;
    let mut visited: Vec<&[u8]> = vec![leaf_der];
    let mut intermediate_count = 0usize;

    for _ in 0..=bag_certs.len() {
        for anchor in trust_anchors {
            if anchor.tbs_certificate().subject() == current.tbs_certificate().issuer()
                && check_cert_validity(anchor, validation_time).is_ok()
                && check_ca_extensions(anchor, intermediate_count, true).is_ok()
                && verify_x509_signature(current, anchor).is_ok()
                && (anchor.tbs_certificate().subject() != anchor.tbs_certificate().issuer()
                    || verify_x509_signature(anchor, anchor).is_ok())
            {
                return Ok(());
            }
        }

        if current.tbs_certificate().subject() == current.tbs_certificate().issuer() {
            return Err(Error::InvalidRdpSignature(
                "signer certificate chain is not anchored in a trusted root",
            ));
        }

        let mut next_issuer: Option<(&[u8], &Certificate)> = None;
        let mut last_error: Option<Error> = None;
        for (cand_der, cand_cert) in bag_certs {
            if cand_cert.tbs_certificate().subject() != current.tbs_certificate().issuer() {
                continue;
            }
            if visited.contains(cand_der) {
                last_error = Some(Error::InvalidRdpSignature("cyclic certificate chain in PKCS#7"));
                continue;
            }
            if let Err(err) = check_cert_validity(cand_cert, validation_time)
                .and_then(|()| check_ca_extensions(cand_cert, intermediate_count, false))
                .and_then(|()| verify_x509_signature(current, cand_cert))
            {
                last_error = Some(err);
                continue;
            }
            next_issuer = Some((cand_der, cand_cert));
            break;
        }

        let Some((issuer_der, issuer_cert)) = next_issuer else {
            return Err(last_error.unwrap_or(Error::InvalidRdpSignature(
                "signer certificate chain is not anchored in a trusted root",
            )));
        };

        visited.push(issuer_der);
        current = issuer_cert;
        intermediate_count += 1;
    }

    Err(Error::InvalidRdpSignature(
        "signer certificate chain is not anchored in a trusted root",
    ))
}

fn check_cert_validity(cert: &Certificate, validation_time: Duration) -> Result<(), Error> {
    let validity = cert.tbs_certificate().validity();
    let not_before = validity.not_before.to_unix_duration();
    let not_after = validity.not_after.to_unix_duration();
    if validation_time < not_before || not_after < validation_time {
        return Err(Error::InvalidRdpSignature(
            "certificate in PKCS#7 chain is expired or not yet valid",
        ));
    }
    Ok(())
}

fn check_leaf_extensions(leaf: &Certificate) -> Result<(), Error> {
    let Some(exts) = leaf.tbs_certificate().extensions() else {
        return Err(Error::InvalidRdpSignature("signer certificate lacks RDP signing EKU"));
    };
    let mut has_rdp_eku = false;
    for ext in exts.iter() {
        let oid = ext.extn_id.as_bytes();
        let val = ext.extn_value.as_bytes();
        if oid == OID_EXT_BASIC_CONSTRAINTS {
            let (is_ca, _) = parse_basic_constraints(val)?;
            if is_ca {
                return Err(Error::InvalidRdpSignature("signer certificate cannot be a CA"));
            }
        } else if oid == OID_EXT_KEY_USAGE {
            let ku = parse_key_usage(val)?;
            if (ku & 0x8000) == 0 {
                return Err(Error::InvalidRdpSignature(
                    "signer certificate KeyUsage lacks digitalSignature",
                ));
            }
        } else if oid == OID_EXT_EKU {
            let eku_oids = parse_extended_key_usage(val)?;
            has_rdp_eku = eku_oids
                .iter()
                .any(|&u| u == OID_EKU_SERVER_AUTH || u == OID_EKU_CODE_SIGNING || u == OID_EKU_MS_RDP_SIGNING);
        } else if ext.critical && oid != OID_EXT_SAN && oid != OID_EXT_CERT_POLICIES {
            return Err(Error::InvalidRdpSignature(
                "unrecognized critical extension in signer certificate",
            ));
        }
    }
    if !has_rdp_eku {
        return Err(Error::InvalidRdpSignature("signer certificate lacks RDP signing EKU"));
    }
    Ok(())
}

fn check_ca_extensions(ca: &Certificate, intermediate_count: usize, is_trust_anchor: bool) -> Result<(), Error> {
    let Some(exts) = ca.tbs_certificate().extensions() else {
        return Err(Error::InvalidRdpSignature(
            "CA certificate lacks BasicConstraints extension",
        ));
    };
    let mut has_ca_basic_constraints = false;
    for ext in exts.iter() {
        let oid = ext.extn_id.as_bytes();
        let val = ext.extn_value.as_bytes();
        if oid == OID_EXT_BASIC_CONSTRAINTS {
            let (is_ca, path_len) = parse_basic_constraints(val)?;
            if !is_ca {
                return Err(Error::InvalidRdpSignature("issuer certificate is not a CA"));
            }
            if let Some(max_len) = path_len
                && max_len < intermediate_count
            {
                return Err(Error::InvalidRdpSignature(
                    "CA pathLenConstraint exceeded in PKCS#7 chain",
                ));
            }
            has_ca_basic_constraints = true;
        } else if oid == OID_EXT_KEY_USAGE {
            let ku = parse_key_usage(val)?;
            if (ku & 0x0400) == 0 {
                return Err(Error::InvalidRdpSignature("CA certificate KeyUsage lacks keyCertSign"));
            }
        } else if oid == OID_EXT_EKU && !is_trust_anchor {
            let eku_oids = parse_extended_key_usage(val)?;
            let allowed = eku_oids.iter().any(|&u| {
                u == OID_EKU_SERVER_AUTH || u == OID_EKU_CODE_SIGNING || u == OID_EKU_MS_RDP_SIGNING || u == OID_EKU_ANY
            });
            if !allowed {
                return Err(Error::InvalidRdpSignature(
                    "intermediate CA ExtendedKeyUsage does not permit RDP signing",
                ));
            }
        } else if ext.critical && oid != OID_EXT_SAN && oid != OID_EXT_CERT_POLICIES {
            return Err(Error::InvalidRdpSignature(
                "unrecognized critical extension in CA certificate",
            ));
        }
    }
    if !has_ca_basic_constraints {
        return Err(Error::InvalidRdpSignature(
            "CA certificate lacks BasicConstraints extension",
        ));
    }
    Ok(())
}

fn parse_basic_constraints(extn_value: &[u8]) -> Result<(bool, Option<usize>), Error> {
    let (tag, mut rest, trailing) =
        read_der_tlv(extn_value).ok_or(Error::InvalidRdpSignature("malformed BasicConstraints"))?;
    if tag != 0x30 || !trailing.is_empty() {
        return Err(Error::InvalidRdpSignature("malformed BasicConstraints"));
    }
    let mut is_ca = false;
    let mut path_len = None;
    if !rest.is_empty() {
        let (first_tag, first_val, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed BasicConstraints"))?;
        if first_tag == 0x01 {
            if first_val.len() != 1 {
                return Err(Error::InvalidRdpSignature("malformed BasicConstraints cA boolean"));
            }
            is_ca = first_val[0] != 0;
            rest = next_rest;
        }
    }
    if !rest.is_empty() {
        let (len_tag, len_val, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed BasicConstraints pathLen"))?;
        if len_tag != 0x02
            || !next_rest.is_empty()
            || len_val.is_empty()
            || len_val.len() > 4
            || (len_val[0] & 0x80) != 0
            || !is_ca
        {
            return Err(Error::InvalidRdpSignature("malformed BasicConstraints pathLen"));
        }
        let mut n = 0usize;
        for &b in len_val {
            n = (n << 8) | usize::from(b);
        }
        path_len = Some(n);
    }
    Ok((is_ca, path_len))
}

fn parse_key_usage(extn_value: &[u8]) -> Result<u16, Error> {
    let (tag, bit_string, trailing) =
        read_der_tlv(extn_value).ok_or(Error::InvalidRdpSignature("malformed KeyUsage"))?;
    if tag != 0x03 || !trailing.is_empty() || bit_string.is_empty() || bit_string[0] > 7 {
        return Err(Error::InvalidRdpSignature("malformed KeyUsage"));
    }
    let bytes = &bit_string[1..];
    if bytes.is_empty() {
        return Ok(0);
    }
    let b0 = u16::from(bytes[0]);
    let b1 = bytes.get(1).copied().map_or(0, u16::from);
    Ok((b0 << 8) | b1)
}

fn parse_extended_key_usage(extn_value: &[u8]) -> Result<Vec<&[u8]>, Error> {
    let (tag, mut rest, trailing) =
        read_der_tlv(extn_value).ok_or(Error::InvalidRdpSignature("malformed ExtendedKeyUsage"))?;
    if tag != 0x30 || !trailing.is_empty() || rest.is_empty() {
        return Err(Error::InvalidRdpSignature("malformed ExtendedKeyUsage"));
    }
    let mut oids = Vec::new();
    while !rest.is_empty() {
        let (oid_tag, oid_bytes, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed ExtendedKeyUsage OID"))?;
        if oid_tag != 0x06 || oid_bytes.is_empty() {
            return Err(Error::InvalidRdpSignature("malformed ExtendedKeyUsage OID"));
        }
        oids.push(oid_bytes);
        rest = next_rest;
    }
    Ok(oids)
}

fn verify_x509_signature(child: &Certificate, issuer: &Certificate) -> Result<(), Error> {
    let sig_oid = child.signature_algorithm().oid.as_bytes();
    if child.tbs_certificate().signature().oid.as_bytes() != sig_oid || child.signature().unused_bits() != 0 {
        return Err(Error::InvalidRdpSignature("certificate signature algorithm mismatch"));
    }
    let hash_alg = match sig_oid {
        OID_SHA256_WITH_RSA => HashAlgorithm::SHA2_256,
        OID_SHA384_WITH_RSA => HashAlgorithm::SHA2_384,
        OID_SHA512_WITH_RSA => HashAlgorithm::SHA2_512,
        _ => {
            return Err(Error::InvalidRdpSignature(
                "unsupported certificate signature algorithm",
            ));
        }
    };
    let spki_der = issuer
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|_| Error::InvalidRdpSignature("failed to encode issuer SPKI"))?;
    let issuer_pk =
        PublicKey::from_der(&spki_der).map_err(|_| Error::InvalidRdpSignature("unsupported issuer public key"))?;
    let tbs_der = child
        .tbs_certificate()
        .to_der()
        .map_err(|_| Error::InvalidRdpSignature("failed to encode TBS certificate"))?;
    let sig_bytes = child.signature().raw_bytes();
    SignatureAlgorithm::RsaPkcs1v15(hash_alg)
        .verify(&issuer_pk, &tbs_der, sig_bytes)
        .map_err(|_| Error::InvalidRdpSignature("certificate chain signature verification failed"))
}

fn encode_der_set(content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 5);
    out.push(0x31);
    let len = u32::try_from(content.len()).unwrap_or(u32::MAX);
    let [_, b2, b1, b0] = len.to_be_bytes();
    if len < 0x80 {
        out.push(b0);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x81, b0]);
    } else if len <= 0xffff {
        out.extend_from_slice(&[0x82, b1, b0]);
    } else {
        out.extend_from_slice(&[0x83, b2, b1, b0]);
    }
    out.extend_from_slice(content);
    out
}

fn parse_pkcs7_signed_data(der: &[u8]) -> Result<ParsedPkcs7<'_>, Error> {
    let (tag, content, trailing) = read_der_tlv(der).ok_or(Error::InvalidRdpSignature("invalid ContentInfo DER"))?;
    if tag != 0x30 || !trailing.is_empty() {
        return Err(Error::InvalidRdpSignature("ContentInfo is not a SEQUENCE"));
    }
    let (oid_tag, oid_bytes, rest) =
        read_der_tlv(content).ok_or(Error::InvalidRdpSignature("missing ContentInfo OID"))?;
    if oid_tag != 0x06 || oid_bytes != OID_SIGNED_DATA {
        return Err(Error::InvalidRdpSignature("ContentInfo is not PKCS#7 signedData"));
    }
    let (ctx_tag, ctx_body, ctx_rem) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing signedData [0]"))?;
    if ctx_tag != 0xa0 || !ctx_rem.is_empty() {
        return Err(Error::InvalidRdpSignature("unexpected signedData wrapper tag"));
    }
    let (signed_data_tag, mut signed_data_body, signed_data_tail) =
        read_der_tlv(ctx_body).ok_or(Error::InvalidRdpSignature("invalid SignedData SEQUENCE"))?;
    if signed_data_tag != 0x30 || !signed_data_tail.is_empty() {
        return Err(Error::InvalidRdpSignature("SignedData is not a SEQUENCE"));
    }

    // 1. version INTEGER
    let (_, _, rest) =
        read_der_tlv(signed_data_body).ok_or(Error::InvalidRdpSignature("missing SignedData version"))?;
    // 2. digestAlgorithms SET
    let (_, _, rest) = read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignedData digestAlgorithms"))?;
    // 3. encapContentInfo SEQUENCE
    let (content_info_tag, content_info_body, mut rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignedData encapContentInfo"))?;
    if content_info_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("encapContentInfo is not a SEQUENCE"));
    }
    let (content_type_tag, content_type_oid, _) =
        read_der_tlv(content_info_body).ok_or(Error::InvalidRdpSignature("missing encapContentInfo OID"))?;
    if content_type_tag != 0x06 || content_type_oid != OID_ID_DATA {
        return Err(Error::InvalidRdpSignature("encapContentInfo is not id-data"));
    }

    let mut certs = Vec::new();
    let mut found_signer_infos = false;
    while !rest.is_empty() {
        let (field_tag, field_body, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed SignedData field"))?;
        if field_tag == 0xa0 {
            // certificates [0] IMPLICIT CertificateSet
            let mut cert_cursor = field_body;
            while !cert_cursor.is_empty() {
                let (c_tag, _, c_next) =
                    read_der_tlv(cert_cursor).ok_or(Error::InvalidRdpSignature("malformed certificate in PKCS#7"))?;
                if c_tag != 0x30 {
                    return Err(Error::InvalidRdpSignature("unexpected non-X.509 certificate in PKCS#7"));
                }
                let consumed = cert_cursor.len() - c_next.len();
                certs.push(&cert_cursor[..consumed]);
                cert_cursor = c_next;
            }
            rest = next_rest;
        } else if field_tag == 0xa1 {
            // crls [1] IMPLICIT
            rest = next_rest;
        } else if field_tag == 0x31 {
            // signerInfos SET
            if !next_rest.is_empty() {
                return Err(Error::InvalidRdpSignature("trailing data after signerInfos"));
            }
            signed_data_body = field_body;
            found_signer_infos = true;
            break;
        } else {
            return Err(Error::InvalidRdpSignature("unexpected tag in SignedData"));
        }
    }

    if !found_signer_infos || certs.is_empty() {
        return Err(Error::InvalidRdpSignature("no certificates in PKCS#7 SignedData"));
    }

    let (signer_info_tag, signer_info_body, _) =
        read_der_tlv(signed_data_body).ok_or(Error::InvalidRdpSignature("missing SignerInfo in signerInfos"))?;
    if signer_info_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("SignerInfo is not a SEQUENCE"));
    }
    // version
    let (_, _, rest) =
        read_der_tlv(signer_info_body).ok_or(Error::InvalidRdpSignature("missing SignerInfo version"))?;
    // sid (IssuerAndSerialNumber or subjectKeyIdentifier)
    let (identifier_tag, identifier_body, rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo sid"))?;
    let signer_id = if identifier_tag == 0x30 {
        let (issuer_tag, _, after_issuer) =
            read_der_tlv(identifier_body).ok_or(Error::InvalidRdpSignature("missing SignerInfo issuer"))?;
        if issuer_tag != 0x30 {
            return Err(Error::InvalidRdpSignature("invalid SignerInfo issuer"));
        }
        let issuer_len = identifier_body.len() - after_issuer.len();
        let issuer_der = &identifier_body[..issuer_len];
        let (serial_tag, serial_bytes, after_serial) =
            read_der_tlv(after_issuer).ok_or(Error::InvalidRdpSignature("missing SignerInfo serialNumber"))?;
        if serial_tag != 0x02 || !after_serial.is_empty() || serial_bytes.is_empty() {
            return Err(Error::InvalidRdpSignature("invalid SignerInfo serialNumber"));
        }
        SignerId::IssuerAndSerialNumber {
            issuer_der,
            serial_bytes,
        }
    } else if identifier_tag == 0x80 {
        if identifier_body.is_empty() {
            return Err(Error::InvalidRdpSignature("empty SignerInfo subjectKeyIdentifier"));
        }
        SignerId::SubjectKeyIdentifier(identifier_body)
    } else {
        return Err(Error::InvalidRdpSignature("unsupported SignerInfo sid choice"));
    };

    // digestAlgorithm AlgorithmIdentifier
    let (digest_alg_tag, digest_alg_body, mut rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo digestAlgorithm"))?;
    if digest_alg_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("digestAlgorithm is not a SEQUENCE"));
    }
    let (digest_oid_tag, digest_oid, _) =
        read_der_tlv(digest_alg_body).ok_or(Error::InvalidRdpSignature("missing digestAlgorithm OID"))?;
    if digest_oid_tag != 0x06 {
        return Err(Error::InvalidRdpSignature("invalid digestAlgorithm OID"));
    }

    let mut signed_attrs_raw = None;
    let (next_tag, next_body, next_rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("truncated SignerInfo"))?;
    let sig_alg_body = if next_tag == 0xa0 {
        signed_attrs_raw = Some(next_body);
        let (alg_seq_tag, alg_seq_body, after_sig_alg) =
            read_der_tlv(next_rest).ok_or(Error::InvalidRdpSignature("missing signatureAlgorithm"))?;
        if alg_seq_tag != 0x30 {
            return Err(Error::InvalidRdpSignature("signatureAlgorithm is not a SEQUENCE"));
        }
        rest = after_sig_alg;
        alg_seq_body
    } else if next_tag == 0x30 {
        rest = next_rest;
        next_body
    } else {
        return Err(Error::InvalidRdpSignature("invalid SignerInfo signatureAlgorithm"));
    };

    let (alg_oid_tag, sig_alg_oid, _) =
        read_der_tlv(sig_alg_body).ok_or(Error::InvalidRdpSignature("missing signatureAlgorithm OID"))?;
    if alg_oid_tag != 0x06 {
        return Err(Error::InvalidRdpSignature("invalid signatureAlgorithm OID"));
    }

    let (sig_tag, sig_bytes, _) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo signature"))?;
    if sig_tag != 0x04 {
        return Err(Error::InvalidRdpSignature(
            "SignerInfo signature is not an OCTET STRING",
        ));
    }

    Ok(ParsedPkcs7 {
        certs_der: certs,
        signer_id,
        digest_oid,
        signed_attrs_raw,
        sig_alg_oid,
        signature_bytes: sig_bytes,
    })
}

fn read_der_tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let &tag = input.first()?;
    let &len_byte = input.get(1)?;
    let (len, header_len) = if (len_byte & 0x80) == 0 {
        (usize::from(len_byte), 2)
    } else {
        let num_octets = usize::from(len_byte & 0x7f);
        if num_octets == 0 || num_octets > 4 || input.len() < 2 + num_octets {
            return None;
        }
        let mut len = 0usize;
        for &b in &input[2..2 + num_octets] {
            len = (len << 8) | usize::from(b);
        }
        if len < 0x80 || (num_octets > 1 && input[2] == 0) {
            return None;
        }
        (len, 2 + num_octets)
    };
    if input.len() < header_len + len {
        return None;
    }
    Some((tag, &input[header_len..header_len + len], &input[header_len + len..]))
}
