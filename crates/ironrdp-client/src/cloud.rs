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
        let http = Self::client_builder()?
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

    /// Discover AVD workspace feed URLs from an RDWeb `api/arm/feeddiscovery` endpoint.
    ///
    /// Supply an Entra bearer token for the AVD RDWeb resource (e.g. `https://www.wvd.microsoft.com`).
    pub async fn discover_avd_feeds(
        &self,
        discovery_url: Url,
        access_token: &str,
    ) -> Result<Vec<AvdFeedSubscription>, Error> {
        validate_avd_service_url(&discovery_url)?;
        let body = self
            .get_with_accept(
                discovery_url,
                access_token,
                "application/x-msts-radc-discovery+xml, application/xml, text/xml",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let xml = core::str::from_utf8(&body).map_err(|_| Error::InvalidXml("response is not valid UTF-8"))?;
        parse_avd_feed_discovery_xml(xml)
    }

    /// Fetch and parse an AVD workspace webfeed (`application/x-msts-radc+xml`).
    pub async fn fetch_avd_workspace_feed(&self, feed_url: Url, access_token: &str) -> Result<AvdWorkspaceFeed, Error> {
        validate_avd_service_url(&feed_url)?;
        let body = self
            .get_with_accept(
                feed_url,
                access_token,
                "application/x-msts-radc+xml, application/xml, text/xml",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let xml = core::str::from_utf8(&body).map_err(|_| Error::InvalidXml("response is not valid UTF-8"))?;
        parse_avd_workspace_feed_xml(xml)
    }

    /// Download a signed `.rdp` file from an AVD workspace resource URL and verify its signature.
    pub async fn download_avd_rdp_file(
        &self,
        rdp_url: Url,
        access_token: &str,
    ) -> Result<(String, VerifiedRdpSignature), Error> {
        validate_avd_service_url(&rdp_url)?;
        let body = self
            .get_with_accept(
                rdp_url,
                access_token,
                "application/x-rdp, text/plain, */*",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        let rdp_text =
            String::from_utf8(body).map_err(|_| Error::InvalidRdpSignature("RDP file is not valid UTF-8"))?;
        let verified = verify_rdp_signature(&rdp_text)?;
        Ok((rdp_text, verified))
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

fn validate_avd_service_url(url: &Url) -> Result<(), Error> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
    {
        return Err(Error::InvalidLaunchUrl);
    }
    Ok(())
}

/// Parse an RDWeb `/api/arm/feeddiscovery` XML document into workspace feed subscriptions.
pub fn parse_avd_feed_discovery_xml(xml: &str) -> Result<Vec<AvdFeedSubscription>, Error> {
    if xml.len() > MAX_PAGE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    let mut subscriptions = Vec::new();
    for tag in find_xml_open_tags(xml) {
        let Some(feed_url_str) = xml_attr(tag, "FeedURL") else {
            continue;
        };
        let feed_url = Url::parse(&feed_url_str).map_err(|_| Error::InvalidXml("invalid FeedURL"))?;
        validate_avd_service_url(&feed_url).map_err(|_| Error::InvalidXml("untrusted FeedURL"))?;
        let tenant_id = xml_attr(tag, "TenantId").unwrap_or_default();
        if tenant_id.is_empty() {
            return Err(Error::InvalidXml("missing TenantId on FeedURL element"));
        }
        let hub_discovery_geo_url = match xml_attr(tag, "HubDiscoveryGeoURL") {
            Some(s) if !s.is_empty() => {
                let u = Url::parse(&s).map_err(|_| Error::InvalidXml("invalid HubDiscoveryGeoURL"))?;
                validate_avd_service_url(&u).map_err(|_| Error::InvalidXml("untrusted HubDiscoveryGeoURL"))?;
                Some(u)
            }
            _ => None,
        };
        let diagnostic_service_url = match xml_attr(tag, "DiagnosticServiceURL") {
            Some(s) if !s.is_empty() => {
                let u = Url::parse(&s).map_err(|_| Error::InvalidXml("invalid DiagnosticServiceURL"))?;
                validate_avd_service_url(&u).map_err(|_| Error::InvalidXml("untrusted DiagnosticServiceURL"))?;
                Some(u)
            }
            _ => None,
        };
        subscriptions.push(AvdFeedSubscription {
            tenant_id,
            tenant_display_name: xml_attr(tag, "TenantDisplayName"),
            feed_url,
            hub_discovery_geo_url,
            diagnostic_service_url,
            activity_hint: xml_attr(tag, "ActivityHint"),
            conditional_access_claims: xml_attr(tag, "ConditionalAccessClaims"),
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
    if !xml.contains("<ResourceCollection") && !xml.contains("<Publisher") {
        return Err(Error::InvalidXml("missing ResourceCollection or Publisher element"));
    }

    let mut workspace_id = None;
    let mut workspace_name = None;
    for tag in find_xml_open_tags(xml) {
        if tag_name(tag).eq_ignore_ascii_case("Publisher") {
            workspace_id = xml_attr(tag, "ID");
            workspace_name = xml_attr(tag, "Name");
            break;
        }
    }

    let mut resources = Vec::new();
    let mut search_rest = xml;
    while let Some(res_start) = find_tag_start(search_rest, "Resource") {
        let after_start = &search_rest[res_start..];
        let Some(open_end) = after_start.find('>') else {
            return Err(Error::InvalidXml("unclosed Resource tag"));
        };
        let open_tag = &after_start[..=open_end];
        let close_needle = "</Resource>";
        let (inner, next_offset) = if open_tag.ends_with("/>") {
            ("", res_start + open_end + 1)
        } else {
            let Some(close_pos) = after_start[open_end + 1..].find(close_needle) else {
                return Err(Error::InvalidXml("missing closing </Resource> tag"));
            };
            let body_start = open_end + 1;
            let body_end = body_start + close_pos;
            (
                &after_start[body_start..body_end],
                res_start + body_end + close_needle.len(),
            )
        };
        search_rest = &search_rest[next_offset..];

        let id = xml_attr(open_tag, "ID").ok_or(Error::InvalidXml("Resource missing ID attribute"))?;
        let title = xml_attr(open_tag, "Title").unwrap_or_else(|| id.clone());
        let alias = xml_attr(open_tag, "Alias");
        let kind = match xml_attr(open_tag, "Type").as_deref() {
            Some(t) if t.eq_ignore_ascii_case("RemoteApp") => AvdResourceKind::RemoteApp,
            _ => AvdResourceKind::Desktop,
        };
        let mut arm_path = xml_attr(open_tag, "ArmPath");
        let mut device_state = xml_attr(open_tag, "DeviceState");
        let mut rdp_file_url = None;
        let mut rdp_etag = None;

        for child_tag in find_xml_open_tags(inner) {
            let name = tag_name(child_tag);
            if arm_path.is_none() {
                arm_path = xml_attr(child_tag, "ArmPath");
            }
            if device_state.is_none() {
                device_state = xml_attr(child_tag, "DeviceState");
            }
            if name.eq_ignore_ascii_case("ResourceFile") {
                let ext = xml_attr(child_tag, "FileExtension").unwrap_or_default();
                if ext.is_empty() || ext.eq_ignore_ascii_case(".rdp") || ext.eq_ignore_ascii_case("rdp") {
                    if let Some(url_str) = xml_attr(child_tag, "URL") {
                        let parsed = Url::parse(&url_str).map_err(|_| Error::InvalidXml("invalid ResourceFile URL"))?;
                        validate_avd_service_url(&parsed)
                            .map_err(|_| Error::InvalidXml("untrusted ResourceFile URL"))?;
                        rdp_file_url = Some(parsed);
                    }
                    rdp_etag = xml_attr(child_tag, "ETag");
                }
            }
        }

        let embedded_rdp = extract_xml_element_text(inner, "RDPFile").map(unescape_xml);

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

fn find_tag_start(haystack: &str, name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(pos) = haystack[offset..].find('<') {
        let idx = offset + pos;
        let rest = &haystack[idx + 1..];
        if rest.len() > name.len()
            && rest[..name.len()].eq_ignore_ascii_case(name)
            && matches!(rest.as_bytes()[name.len()], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
        {
            return Some(idx);
        }
        offset = idx + 1;
    }
    None
}

fn find_xml_open_tags(xml: &str) -> Vec<&str> {
    let mut tags = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let after = &rest[start..];
        let Some(end) = after.find('>') else {
            break;
        };
        let tag = &after[..=end];
        if !tag.starts_with("</") && !tag.starts_with("<?") && !tag.starts_with("<!") {
            tags.push(tag);
        }
        rest = &after[end + 1..];
    }
    tags
}

fn tag_name(tag: &str) -> &str {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    inner.split_whitespace().next().unwrap_or("")
}

fn xml_attr(tag: &str, attr_name: &str) -> Option<String> {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    let mut rest = inner;
    while let Some(eq_pos) = rest.find('=') {
        let before = rest[..eq_pos].trim_end();
        let key = before
            .rsplit_once(|c: char| c.is_whitespace())
            .map_or(before, |(_, k)| k);
        let after = rest[eq_pos + 1..].trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            rest = after;
            continue;
        }
        let value_rest = &after[1..];
        let end_quote = value_rest.find(quote)?;
        let raw_val = &value_rest[..end_quote];
        if key.eq_ignore_ascii_case(attr_name) {
            return Some(unescape_xml(raw_val));
        }
        rest = &value_rest[end_quote + 1..];
    }
    None
}

fn extract_xml_element_text<'a>(xml: &'a str, element_name: &str) -> Option<&'a str> {
    let start_idx = find_tag_start(xml, element_name)?;
    let after = &xml[start_idx..];
    let open_end = after.find('>')?;
    if after[..=open_end].ends_with("/>") {
        return None;
    }
    let body = &after[open_end + 1..];
    let close_tag = format!("</{element_name}>");
    let close_pos = body.find(&close_tag)?;
    Some(&body[..close_pos])
}

fn unescape_xml(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Sensitive `.rdp` settings that must be covered by `signscope` whenever present in a signed `.rdp` file.
const SENSITIVE_RDP_SETTINGS: &[&str] = &[
    "full address",
    "alternate full address",
    "gatewayhostname",
    "gatewayusagemethod",
    "gatewayprofileusagemethod",
    "gatewaycredentialssource",
    "loadbalanceinfo",
    "remoteapplicationprogram",
    "remoteapplicationmode",
    "alternate shell",
    "armpath",
    "aadtenantid",
    "enablerdsaadauth",
    "gatewaybrokeringtype",
    "wvd endpoint pool",
    "workspace id",
];

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

/// Verify the `signscope` and detached PKCS#7 `signature` of an `.rdp` file.
///
/// Enforces sensitive-setting coverage (`VerifySensitiveSettings` in `RDCoreSdk.dll`), verifies
/// the detached PKCS#7 RSA signature over the canonical UTF-16LE `signscope` lines, and validates
/// the embedded X.509 issuer chain.
pub fn verify_rdp_signature(rdp_content: &str) -> Result<VerifiedRdpSignature, Error> {
    let mut lines_by_key: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for raw_line in rdp_content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, _)) = trimmed.split_once(':') else {
            continue;
        };
        lines_by_key.insert(key.trim().to_ascii_lowercase(), trimmed.to_owned());
    }

    let signscope_line = lines_by_key
        .get("signscope")
        .ok_or(Error::InvalidRdpSignature("missing signscope property"))?;
    let signscope_value = rdp_line_value(signscope_line).ok_or(Error::InvalidRdpSignature("malformed signscope"))?;
    let signed_scope: Vec<String> = signscope_value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if signed_scope.is_empty() {
        return Err(Error::InvalidRdpSignature("signscope is empty"));
    }

    for sensitive in SENSITIVE_RDP_SETTINGS {
        if lines_by_key.contains_key(*sensitive) && !signed_scope.iter().any(|s| s.eq_ignore_ascii_case(sensitive)) {
            return Err(Error::InvalidRdpSignature(
                "sensitive .rdp setting is not covered by signscope",
            ));
        }
    }

    let mut canonical = String::new();
    for scope_key in &signed_scope {
        let line = lines_by_key
            .get(&scope_key.to_ascii_lowercase())
            .ok_or(Error::InvalidRdpSignature("property listed in signscope is missing"))?;
        canonical.push_str(line);
        canonical.push_str("\r\n");
    }
    canonical.push_str(signscope_line);
    canonical.push_str("\r\n");

    let mut canonical_utf16le: Vec<u8> = Vec::with_capacity((canonical.len() + 1) * 2);
    for unit in canonical.encode_utf16() {
        canonical_utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    canonical_utf16le.extend_from_slice(&[0, 0]);

    let sig_line = lines_by_key
        .get("signature")
        .ok_or(Error::InvalidRdpSignature("missing signature property"))?;
    let sig_b64: String = rdp_line_value(sig_line)
        .ok_or(Error::InvalidRdpSignature("malformed signature property"))?
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let raw_sig = STANDARD
        .decode(&sig_b64)
        .map_err(|_| Error::InvalidRdpSignature("invalid base64 in signature"))?;
    if raw_sig.len() <= 12 {
        return Err(Error::InvalidRdpSignature("signature envelope is truncated"));
    }
    let declared_len = usize::try_from(u32::from_le_bytes([raw_sig[8], raw_sig[9], raw_sig[10], raw_sig[11]]))
        .map_err(|_| Error::InvalidRdpSignature("invalid signature length"))?;
    let pkcs7_der = &raw_sig[12..];
    if pkcs7_der.len() < declared_len {
        return Err(Error::InvalidRdpSignature(
            "PKCS#7 payload is shorter than header length",
        ));
    }
    let pkcs7_der = &pkcs7_der[..declared_len];

    let (certs_der, digest_oid, signed_attrs_raw, signature_bytes) = parse_pkcs7_signed_data(pkcs7_der)?;
    let hash_alg = match digest_oid.as_slice() {
        // 2.16.840.1.101.3.4.2.1 (id-sha256)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01] => HashAlgorithm::SHA2_256,
        // 2.16.840.1.101.3.4.2.2 (id-sha384)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02] => HashAlgorithm::SHA2_384,
        // 2.16.840.1.101.3.4.2.3 (id-sha512)
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03] => HashAlgorithm::SHA2_512,
        // 1.3.14.3.2.26 (id-sha1)
        [0x2b, 0x0e, 0x03, 0x02, 0x1a] => HashAlgorithm::SHA1,
        _ => return Err(Error::InvalidRdpSignature("unsupported PKCS#7 digest algorithm")),
    };

    let signed_payload = if let Some(attrs_body) = signed_attrs_raw {
        let expected_digest = hash_alg.digest(&canonical_utf16le);
        if !subslice_contains(attrs_body, &expected_digest) {
            return Err(Error::InvalidRdpSignature("PKCS#7 messageDigest attribute mismatch"));
        }
        encode_der_set(attrs_body)
    } else {
        canonical_utf16le
    };

    let parsed_certs: Vec<(&[u8], Certificate)> = certs_der
        .iter()
        .filter_map(|&der| Certificate::from_der(der).ok().map(|c| (der, c)))
        .collect();
    if parsed_certs.is_empty() {
        return Err(Error::InvalidRdpSignature("no valid X.509 certificates in PKCS#7"));
    }

    let sig_algo = SignatureAlgorithm::RsaPkcs1v15(hash_alg);
    for (cert_der, cert) in &parsed_certs {
        let Ok(spki_der) = cert.tbs_certificate().subject_public_key_info().to_der() else {
            continue;
        };
        let Ok(public_key) = PublicKey::from_der(&spki_der) else {
            continue;
        };
        if sig_algo.verify(&public_key, &signed_payload, signature_bytes).is_ok() {
            verify_embedded_issuer_chain(cert, &parsed_certs)?;
            let signer_subject = cert.tbs_certificate().subject().to_string();
            return Ok(VerifiedRdpSignature {
                signed_scope,
                signer_subject,
                signer_certificate_der: (*cert_der).to_vec(),
            });
        }
    }

    Err(Error::InvalidRdpSignature("RSA signature verification failed"))
}

fn verify_embedded_issuer_chain(leaf: &Certificate, certs: &[(&[u8], Certificate)]) -> Result<(), Error> {
    let mut current = leaf;
    for _ in 0..certs.len() {
        if current.tbs_certificate().subject() == current.tbs_certificate().issuer() {
            return Ok(());
        }
        let Some((_, issuer_cert)) = certs
            .iter()
            .find(|(_, cand)| cand.tbs_certificate().subject() == current.tbs_certificate().issuer())
        else {
            break;
        };
        let spki_der = issuer_cert
            .tbs_certificate()
            .subject_public_key_info()
            .to_der()
            .map_err(|_| Error::InvalidRdpSignature("failed to encode issuer SPKI"))?;
        let issuer_pk =
            PublicKey::from_der(&spki_der).map_err(|_| Error::InvalidRdpSignature("unsupported issuer public key"))?;
        let tbs_der = current
            .tbs_certificate()
            .to_der()
            .map_err(|_| Error::InvalidRdpSignature("failed to encode TBS certificate"))?;
        let sig_bytes = current.signature().raw_bytes();
        let verified = [
            HashAlgorithm::SHA2_256,
            HashAlgorithm::SHA2_384,
            HashAlgorithm::SHA2_512,
            HashAlgorithm::SHA1,
        ]
        .into_iter()
        .any(|alg| {
            SignatureAlgorithm::RsaPkcs1v15(alg)
                .verify(&issuer_pk, &tbs_der, sig_bytes)
                .is_ok()
        });
        if !verified {
            return Err(Error::InvalidRdpSignature(
                "certificate chain signature verification failed",
            ));
        }
        current = issuer_cert;
    }
    Ok(())
}

fn rdp_line_value(line: &str) -> Option<&str> {
    let mut parts = line.splitn(3, ':');
    let _key = parts.next()?;
    let _ty = parts.next()?;
    parts.next()
}

fn subslice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn encode_der_set(content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 5);
    out.push(0x31);
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, (len & 0xff) as u8]);
    }
    out.extend_from_slice(content);
    out
}

#[expect(clippy::type_complexity)]
fn parse_pkcs7_signed_data(der: &[u8]) -> Result<(Vec<&[u8]>, Vec<u8>, Option<&[u8]>, &[u8]), Error> {
    let (tag, content, _) = read_der_tlv(der).ok_or(Error::InvalidRdpSignature("invalid ContentInfo DER"))?;
    if tag != 0x30 {
        return Err(Error::InvalidRdpSignature("ContentInfo is not a SEQUENCE"));
    }
    let (oid_tag, oid_bytes, rest) =
        read_der_tlv(content).ok_or(Error::InvalidRdpSignature("missing ContentInfo OID"))?;
    // 1.2.840.113549.1.7.2 (signedData)
    if oid_tag != 0x06 || oid_bytes != [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02] {
        return Err(Error::InvalidRdpSignature("ContentInfo is not PKCS#7 signedData"));
    }
    let (ctx_tag, ctx_body, _) = read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing signedData [0]"))?;
    if ctx_tag != 0xa0 {
        return Err(Error::InvalidRdpSignature("unexpected signedData wrapper tag"));
    }
    let (sd_tag, mut sd_body, _) =
        read_der_tlv(ctx_body).ok_or(Error::InvalidRdpSignature("invalid SignedData SEQUENCE"))?;
    if sd_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("SignedData is not a SEQUENCE"));
    }

    // 1. version INTEGER
    let (_, _, rest) = read_der_tlv(sd_body).ok_or(Error::InvalidRdpSignature("missing SignedData version"))?;
    // 2. digestAlgorithms SET
    let (_, _, rest) = read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignedData digestAlgorithms"))?;
    // 3. encapContentInfo SEQUENCE
    let (_, _, mut rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignedData encapContentInfo"))?;

    let mut certs = Vec::new();
    while !rest.is_empty() {
        let (field_tag, field_body, next_rest) =
            read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("malformed SignedData field"))?;
        if field_tag == 0xa0 {
            // certificates [0] IMPLICIT CertificateSet
            let mut cert_cursor = field_body;
            while !cert_cursor.is_empty() {
                let Some((c_tag, _, c_next)) = read_der_tlv(cert_cursor) else {
                    break;
                };
                let consumed = cert_cursor.len() - c_next.len();
                if c_tag == 0x30 {
                    certs.push(&cert_cursor[..consumed]);
                }
                cert_cursor = c_next;
            }
            rest = next_rest;
        } else if field_tag == 0xa1 {
            // crls [1] IMPLICIT
            rest = next_rest;
        } else if field_tag == 0x31 {
            // signerInfos SET
            sd_body = field_body;
            break;
        } else {
            return Err(Error::InvalidRdpSignature("unexpected tag in SignedData"));
        }
    }

    if certs.is_empty() {
        return Err(Error::InvalidRdpSignature("no certificates in PKCS#7 SignedData"));
    }

    let (si_tag, si_body, _) =
        read_der_tlv(sd_body).ok_or(Error::InvalidRdpSignature("missing SignerInfo in signerInfos"))?;
    if si_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("SignerInfo is not a SEQUENCE"));
    }
    // version
    let (_, _, rest) = read_der_tlv(si_body).ok_or(Error::InvalidRdpSignature("missing SignerInfo version"))?;
    // sid (IssuerAndSerialNumber or subjectKeyIdentifier)
    let (_, _, rest) = read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo sid"))?;
    // digestAlgorithm AlgorithmIdentifier
    let (da_tag, da_body, mut rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo digestAlgorithm"))?;
    if da_tag != 0x30 {
        return Err(Error::InvalidRdpSignature("digestAlgorithm is not a SEQUENCE"));
    }
    let (da_oid_tag, da_oid, _) =
        read_der_tlv(da_body).ok_or(Error::InvalidRdpSignature("missing digestAlgorithm OID"))?;
    if da_oid_tag != 0x06 {
        return Err(Error::InvalidRdpSignature("invalid digestAlgorithm OID"));
    }

    let mut signed_attrs_raw = None;
    let (next_tag, next_body, next_rest) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("truncated SignerInfo"))?;
    if next_tag == 0xa0 {
        signed_attrs_raw = Some(next_body);
        // signatureAlgorithm
        let (_, _, after_sig_alg) =
            read_der_tlv(next_rest).ok_or(Error::InvalidRdpSignature("missing signatureAlgorithm"))?;
        rest = after_sig_alg;
    } else {
        // next_tag was signatureAlgorithm
        rest = next_rest;
    }

    let (sig_tag, sig_bytes, _) =
        read_der_tlv(rest).ok_or(Error::InvalidRdpSignature("missing SignerInfo signature"))?;
    if sig_tag != 0x04 {
        return Err(Error::InvalidRdpSignature(
            "SignerInfo signature is not an OCTET STRING",
        ));
    }

    Ok((certs, da_oid.to_vec(), signed_attrs_raw, sig_bytes))
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
        (len, 2 + num_octets)
    };
    if input.len() < header_len + len {
        return None;
    }
    Some((tag, &input[header_len..header_len + len], &input[header_len + len..]))
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
            http: CloudPcClient::client_builder().unwrap().no_proxy().build().unwrap(),
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

    #[test]
    fn avd_cloud_environment_feed_discovery_urls() {
        assert_eq!(
            AvdCloudEnvironment::Public
                .feed_discovery_url(Some("4c5a5ec1-9ac5-4612-9b4c-6bed178bb65a"))
                .unwrap()
                .as_str(),
            "https://www.wvd.microsoft.com/api/arm/feeddiscovery?aadtenant=4c5a5ec1-9ac5-4612-9b4c-6bed178bb65a"
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
    }

    #[test]
    fn parse_avd_feed_discovery_and_workspace_feed() {
        let discovery_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<FeedDiscovery>
  <Publisher TenantId="ada6b91e-20af-4bb3-aea1-111328fd1811"
             TenantDisplayName="AvdLabWorkspace"
             FeedURL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/ada6b91e-20af-4bb3-aea1-111328fd1811/webfeed?geo=CA&amp;showInFeedMonikers=CloudPC"
             HubDiscoveryGeoURL="https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/hubdiscovery?resourceId=47ddc51c-2ce6-466a-8ee9-453f7b2072ee"
             DiagnosticServiceURL="https://rdweb-g-ca-r1.wvd.microsoft.com/api/arm/DiagnosticEvents/v1"
             ActivityHint="ms-wvd-hp:47ddc51c-2ce6-466a-8ee9-453f7b2072ee" />
</FeedDiscovery>"#;
        let feeds = parse_avd_feed_discovery_xml(discovery_xml).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].tenant_id, "ada6b91e-20af-4bb3-aea1-111328fd1811");
        assert_eq!(feeds[0].tenant_display_name.as_deref(), Some("AvdLabWorkspace"));
        assert_eq!(
            feeds[0].feed_url.as_str(),
            "https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/ada6b91e-20af-4bb3-aea1-111328fd1811/webfeed?geo=CA&showInFeedMonikers=CloudPC"
        );
        assert!(feeds[0].hub_discovery_geo_url.is_some());
        assert!(feeds[0].diagnostic_service_url.is_some());

        let workspace_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ResourceCollection PubDate="2026-10-26T18:00:00Z" SchemaVersion="2.1">
  <Publisher LastUpdated="2026-10-26T18:00:00Z" Name="AvdLabWorkspace" ID="ada6b91e-20af-4bb3-aea1-111328fd1811" Description="">
    <Resources>
      <Resource ID="20842633-c909-4aeb-3199-08db09107d3b" Alias="SessionDesktop" Title="SessionDesktop" Type="Desktop"
                ArmPath="/subscriptions/6fe9553e-226e-4185-9cc2-da3c31036ca8/resourcegroups/AVD/providers/Microsoft.DesktopVirtualization/applicationgroups/AvdLab-DAG/applications/SessionDesktop"
                DeviceState="Available">
        <HostingTerminalServers>
          <HostingTerminalServer>
            <ResourceFile FileExtension=".rdp" URL="https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/ada6b91e-20af-4bb3-aea1-111328fd1811/rdps/20842633-c909-4aeb-3199-08db09107d3b.rdp?geo=CA" ETag="etag-123" />
          </HostingTerminalServer>
        </HostingTerminalServers>
      </Resource>
    </Resources>
  </Publisher>
</ResourceCollection>"#;
        let ws = parse_avd_workspace_feed_xml(workspace_xml).unwrap();
        assert_eq!(ws.workspace_id.as_deref(), Some("ada6b91e-20af-4bb3-aea1-111328fd1811"));
        assert_eq!(ws.workspace_name.as_deref(), Some("AvdLabWorkspace"));
        assert_eq!(ws.resources.len(), 1);
        assert_eq!(ws.resources[0].id, "20842633-c909-4aeb-3199-08db09107d3b");
        assert_eq!(ws.resources[0].kind, AvdResourceKind::Desktop);
        assert_eq!(ws.resources[0].device_state.as_deref(), Some("Available"));
        assert_eq!(ws.resources[0].rdp_etag.as_deref(), Some("etag-123"));
        assert_eq!(
            ws.resources[0].rdp_file_url.as_ref().unwrap().as_str(),
            "https://rdweb-r1.wvd.microsoft.com/api/arm/feeddiscovery/tenants/ada6b91e-20af-4bb3-aea1-111328fd1811/rdps/20842633-c909-4aeb-3199-08db09107d3b.rdp?geo=CA"
        );
    }

    #[test]
    fn verify_microsoft_signed_avd_rdp_file_and_detect_tampering() {
        let signed_rdp = concat!(
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
            "signature:s:AQABAAEAAABsGwAAMIIbaAYJKoZIhvcNAQcCoIIbWTCCG1UCAQExDTALBglghkgB  ZQMEAgEwCwYJKoZIhvcNAQcBoIIZmTCCA44wggJ2oAMCAQICEAM68eanEamguyhk  sR0J+uUwDQYJKoZIhvcNAQELBQAwYTELMAkGA1UEBhMCVVMxFTATBgNVBAoTDERp  Z2lDZXJ0IEluYzEZMBcGA1UECxMQd3d3LmRpZ2ljZXJ0LmNvbTEgMB4GA1UEAxMX  RGlnaUNlcnQgR2xvYmFsIFJvb3QgRzIwHhcNMTMwODAxMTIwMDAwWhcNMzgwMTE1  MTIwMDAwWjBhMQswCQYDVQQGEwJVUzEVMBMGA1UEChMMRGlnaUNlcnQgSW5jMRkw  FwYDVQQLExB3d3cuZGlnaWNlcnQuY29tMSAwHgYDVQQDExdEaWdpQ2VydCBHbG9i  YWwgUm9vdCBHMjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALs3zTTc  e2vJsmiQrUp1/0a6IQoIjfUZVMn7iNvzrvI6iZE8euarBhprz6wt6F4JJES6Ypp+  1qOofuBUdSAFrFC3nGMabDDc2h8Zsdce3v3X4MuUgzeu7B9DTt17LNK9LqUv5Km4  rTrUmaS2JembawBgkmD/TyFJGPdnkKthBpyP8rrptOmSMmu181foXRvNjB2rlQSV  SfM1LZbjSW3dd+P7SUu0rFUHqY+Vs7Qju0xtRfD2qbKVMLT9TFWMJ0pXFHyCnc1z  ktMWSgYMjFDRjx4Jvheh5iHK/YPlELyDpQrEZyj2cxQUPUZ2w4cUiSE0Ta8PRQym  SaG6u5zFsTODKYUCAwEAAaNCMEAwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8E  BAMCAYYwHQYDVR0OBBYEFE4iVCAYlebjbuYP+vq5Eu0GF485MA0GCSqGSIb3DQEB  CwUAA4IBAQBgZyiUbw5IY+sx3epnGNWJfTzFi0p/6b7bKxffsF9zdyoyEzmBZ0KE  I/JFZzXsiL/4j7BhDDSkriBMhMbb+DXhdtnfpkK7x0QIhn82dCRa2mwNFFk1vfJJ  3bYfybMNRyo9mS+7XLu11CDhmV9TRhXbaJvw8zDVPjHijYSe44ra2pY+NROlX/D5  cFBwR0ERVxlOwI+uBsSVExcvGyWfdfKxjpmhbxOxQXH+iCrITxAgVdfzFEXl4ET0  6oeVMpMO/lNG+iyd/4siuUvZCUWk3qS4mljdG31Sn45ZQ4iBpJ4m1W+t3Q3GN33t  A5Ib5Xdfdu48jcRdVlui2WZuszU35TK2MIIFiTCCBHGgAwIBAgIQCwxrLEZpF7BH  c8ZH1K/AyDANBgkqhkiG9w0BAQwFADBhMQswCQYDVQQGEwJVUzEVMBMGA1UEChMM  RGlnaUNlcnQgSW5jMRkwFwYDVQQLExB3d3cuZGlnaWNlcnQuY29tMSAwHgYDVQQD  ExdEaWdpQ2VydCBHbG9iYWwgUm9vdCBHMjAeFw0yNTA1MjEwMDAwMDBaFw0yOTA2  MTkyMzU5NTlaMFExCzAJBgNVBAYTAlVTMR4wHAYDVQQKExVNaWNyb3NvZnQgQ29y  cG9yYXRpb24xIjAgBgNVBAMTGU1pY3Jvc29mdCBUTFMgUlNBIFJvb3QgRzIwggIi  MA0GCSqGSIb3DQEBAQUAA4ICDwAwggIKAoICAQDf6oufR+EoEHGvQdYZ25JX3mur  5i7erTpgg7cTmKxbuTILe+ufcidrXUCrvhgGk7IN0hLtuHT1fy/qqBeU9jMWV4re  IHwh3bfarN5OZLBazUt18+8CZE3tUtqjjwTokfjX+z8Z/U5FOV7oKcPW8mevswCU  wY3h8EoYmDn6wAmEM0EFAwWr9HXhU6UhklxETOZgV6SQApfH1diTBDJK7YVR7dbF  uqA/Noovb0w5qARpIoQ7dRT32T60qdAHQTiBfkZIHegZ5nC4oKoY3XK/fn21bE4Z  cBGEBBOB1GL9nGvxHN3/7Kfg5seNMUu/8mszzNGMtv6xG6NKqF8OfzF2OD8HR2wB  qKylFNqCsF8fbLyJGsASKst7lx8oLjEWilNMdWb5fQHWwmCqZY8xnnLLzJst5UQZ  k1erbo7C2S5lsHIt56HDoX5JHVln1gnUGBJtwJVFeMnxYGrk9u4GJDtzSloRwj6X  YcB47u8TpzDiSjgt7lgXEyC3NirfCzK0wjixkd0SsEW2fMCxHWKhnd1xEhWWAZ0K  CfWx3bPZ4DhCNPZptsOvFnP+1EP4Q+RY+U+z8+zWPZQ6QDgVqwyG0GTOGmPohJRV  CVq2BLbRPpoVx2QRgNAbgg5N/0WesmUHJR/bmsjG7NZbhVAEnxzLXSCCZ5554t/o  8uhvxCByMIblnXUnNQIDAQABo4IBSzCCAUcwDwYDVR0TAQH/BAUwAwEB/zAdBgNV  HQ4EFgQU3pGGSLehMVkx8UtfB6nciHnaqHYwHwYDVR0jBBgwFoAUTiJUIBiV5uNu  5g/6+rkS7QYXjzkwDgYDVR0PAQH/BAQDAgGGMBMGA1UdJQQMMAoGCCsGAQUFBwMB  MHYGCCsGAQUFBwEBBGowaDAkBggrBgEFBQcwAYYYaHR0cDovL29jc3AuZGlnaWNl  cnQuY29tMEAGCCsGAQUFBzAChjRodHRwOi8vY2FjZXJ0cy5kaWdpY2VydC5jb20v  RGlnaUNlcnRHbG9iYWxSb290RzIuY3J0MEIGA1UdHwQ7MDkwN6A1oDOGMWh0dHA6  Ly9jcmwzLmRpZ2ljZXJ0LmNvbS9EaWdpQ2VydEdsb2JhbFJvb3RHMi5jcmwwEwYD  VR0gBAwwCjAIBgZngQwBAgIwDQYJKoZIhvcNAQEMBQADggEBAAu8tCs3dMVLpzYC  Nsav4RPMipqXG/zjRIzuVADl5EEaRvALdjT/mVViNaqtipwMWmLMQ8DL6kodvWsd  r7EZJWac93luWyWAJIGFx3ktNV9CCXjtn+Jl1cQgUIIQj2o67RiOSImrpgn44YD8  BnUWJyVaj7g6cGwYR/Bj9FMO2RU1IPORPRMBoOL6JAhFVnfRZ6kxQtBX/xomvsVD  2FepY/+v8zrY9ntLEKKXoc9mvmdnCfm1TOerGSu/Ij193sb372M4LN1WxPkJUtrf  44hv1W1r9whBL44+hjGf8XxK9dZhpEZGKO9XurBvktjSdyXte6YpzjtyeRHU4KdU  bTUrpHowgge4MIIFoKADAgECAhMzAAAACxPlZn1Km1WAAAAAAAALMA0GCSqGSIb3  DQEBDAUAMFExCzAJBgNVBAYTAlVTMR4wHAYDVQQKExVNaWNyb3NvZnQgQ29ycG9y  YXRpb24xIjAgBgNVBAMTGU1pY3Jvc29mdCBUTFMgUlNBIFJvb3QgRzIwHhcNMjUw  ODAxMjAwMjU5WhcNMjkwNjAzMjAwMjU5WjBXMQswCQYDVQQGEwJVUzEeMBwGA1UE  ChMVTWljcm9zb2Z0IENvcnBvcmF0aW9uMSgwJgYDVQQDEx9NaWNyb3NvZnQgVExT  IEcyIFJTQSBDQSBPQ1NQIDA0MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKC  AgEAnDokCFoYkvKe1WDN4ub7gEfbRK4r3W0QcCEVRSmq0sfc5sRCsaCNUuBPXDlV  BjZNF8FYZjTihVKf4qT+xt7CwjuGv8qthykG6x1JHV+B27ekrFYa1hKH/V9ASeKg  DGqExk5v3yS4mpV0Wll/rek8ExV4QMzs2u+INkqQ3upjB5TDg2oarpjm8WSAWiln  BOhW9CGmxUaNyN0Cq7wG8GxHeCO7OLC7+01xsN8iw38afvmdqkEq0d5o3ZV0JThm  yRYRxVk9KrFouZM7Zug84fyn/yxrUODAuchxc5Oxd7/tqk0OClXbE5xy4GmtgUza  I/kGyz/tQoqGjA3SDBUl0IZo63hWqi5cCLNNM4BO7jU8L8N6fcFmcmBokPTr9gjs  E9eJkl9cUvSHlXixNuGzQlNuOjc3B8hzdR67XW5nyZp8y0rzxs3wTvWG+xQ1GUY9  l4ZyZN1DNu/UV4IAIgyjOu2zZXBl+YKlurjS6sBoijU3oED1/gKJZqgw+JRHYldH  snubjMeVvOua9saMRInbFuYe/eeJ/IE1NTDqL32JZFKzRhk04ztXTwoYIDHvxf25  sSf2l1k1+wrCRNbQsu1VTR1XSHUS1/KYYBaexAac1K5+LmXHU+QiJRgwY2qQV8WU  tPflTUwNVMcfqT1NCkYJYtAjIlHjjjdMxfQBQJkFgtOisFMCAwEAAaOCAoEwggJ9  MA4GA1UdDwEB/wQEAwIBhjAQBgkrBgEEAYI3FQEEAwIBADAdBgNVHQ4EFgQUVAy8  7Bj3ffED4oS+NGREZ891H2UwEwYDVR0gBAwwCjAIBgZngQwBAgIwEwYDVR0lBAww  CgYIKwYBBQUHAwEwGQYJKwYBBAGCNxQCBAweCgBTAHUAYgBDAEEwEgYDVR0TAQH/  BAgwBgEB/wIBADAfBgNVHSMEGDAWgBTekYZIt6ExWTHxS18HqdyIedqodjCBqwYD  VR0fBIGjMIGgMIGdoIGaoIGXhklodHRwOi8vd3d3Lm1pY3Jvc29mdC5jb20vcGtp  b3BzL2NybC9NaWNyb3NvZnQlMjBUTFMlMjBSU0ElMjBSb290JTIwRzIuY3Jshkpo  dHRwOi8vY3JsMi5taWNyb3NvZnQuY29tL3BraW9wcy9jcmwvTWljcm9zb2Z0JTIw  VExTJTIwUlNBJTIwUm9vdCUyMEcyLmNybDCCARAGCCsGAQUFBwEBBIIBAjCB/zBj  BggrBgEFBQcwAoZXaHR0cDovL3d3dy5taWNyb3NvZnQuY29tL3BraW9wcy9jZXJ0  cy9NaWNyb3NvZnQlMjBUTFMlMjBSU0ElMjBSb290JTIwRzIlMjAtJTIweHNpZ24u  Y3J0MGkGCCsGAQUFBzAChl1odHRwOi8vY2Fpc3N1ZXJzLm1pY3Jvc29mdC5jb20v  cGtpb3BzL2NlcnRzL01pY3Jvc29mdCUyMFRMUyUyMFJTQSUyMFJvb3QlMjBHMiUy  MC0lMjB4c2lnbi5jcnQwLQYIKwYBBQUHMAGGIWh0dHA6Ly9vbmVvY3NwLm1pY3Jv  c29mdC5jb20vb2NzcDANBgkqhkiG9w0BAQwFAAOCAgEAfEhxwrbARat0D8Y/SeL8  D9Z1/c1Q0eBLs7PRqJlf/l01DWwL3dCtYWAYiTji8Lc10rUhOgwOWJk4fNTX3QJN  1Nz17pDje2eTqKzUG1C7xU/m0/G1HvD/mbwjM7DIqKYn1tFb7W8U3ksHGgxQ/Q7f  vutPspgQyyIq5h3XS6euFdEBNgkLhkNPTufFflyja4MuST6UiJmLUtOWP3GYJC83  gvpsdPK82wYhu7hQZ3AkFbYLIo/3uSfkA3yElbdmBKmSVcK9EcchCnJ/ZcBgplBJ  6fYjSCA6jpImqBeMmK8Upz3OTK3Ajow8RPF/+xP5FNd7Mz0QXmTnJz0MTOJx12bO  rvdOtmS6VCu3/QPXmGeYN7IKYyDlsPvYpI06LkIimiL0JlWDMBXHokKzv9A2xTZS  p3NnenNuVKy86YnYpesfbawnUMTI69QcVClNu7beq9fH8R8UZLp/RzFl73l9FMTw  GeSAduc7KACbRG8kV/UH3I5atABYl1axC/oIdXHeG1PI7r3wqyJMvWu6XYpEWTwk  a6q86X2LX0pN14xUsKt2m4AoTUV0ZRBgIfmOeV/akvsXhADbxK6CWYE8J3eeVvcH  yL2+PZHFl4rmnA9ZFlhIbH2kFjLtFsktp5b15P3Fz2Wk3dcA6MkO9J2ypAKAzBvw  KOuwMXc0nsPM18ibNrvFJeIwggi6MIIGoqADAgECAhNDAUHagacuhGWhZnddAAAB  QdqBMA0GCSqGSIb3DQEBDAUAMFcxCzAJBgNVBAYTAlVTMR4wHAYDVQQKExVNaWNy  b3NvZnQgQ29ycG9yYXRpb24xKDAmBgNVBAMTH01pY3Jvc29mdCBUTFMgRzIgUlNB  IENBIE9DU1AgMDQwHhcNMjYxMDEyMTE1NTM5WhcNMjcwMTIwMTA1NTM5WjBsMQsw  CQYDVQQGEwJVUzELMAkGA1UECBMCV0ExEDAOBgNVBAcTB1JlZG1vbmQxHjAcBgNV  BAoTFU1pY3Jvc29mdCBDb3Jwb3JhdGlvbjEeMBwGA1UEAxMVcmRwLnd2ZC5taWNy  b3NvZnQuY29tMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAyhBN9YgW  XMU/3l8Q7Ua1i/POLsxUhOQ+6uu121F9qZQ59tYVB7QDZPefbt0IdpMtnM/eJ2Kd  IFFU6nN3LPS9pkaKiHNYxRh3v1BW768ZLkpBmpIlMbDub7BHPFCP81KBXo6ARABD  TuJ0Z/0gmjZGoule1yhG/RYYgKwYCSXufMqI1cZt3I4q1EyPAYbRH/zqjF5vHWrz  3HHZIuFjTxZ7RkNR3H6J0iWfnjEtrf5TQCONKtEpPGBOJTdYZgoUbmkLMgjDtGrt  ms/qAy5pbgaZth6cYaP1CRyl2gnMxJMgVKWZ6MKCIXtwjZBWm0rmz+SpnVFb2tl5  MMCrCf/mboVLcQIDAQABo4IEaDCCBGQwggEFBgorBgEEAdZ5AgQCBIH2BIHzAPEA  dwBEwr0M6RQOZKXJSgGTClqhuzWXDgDuERaJaCocRNe1ZgAAAaEwAR1pAAAEAwBI  MEYCIQCNJwkxLkRxJOp06tExAemKvZCz9HC+c38iRhzBuCIhJwIhAOnakVENbYkN  LzdPGE+PNyPx76SWAU5mnmfqyTo/kdwrAHYAABpdGhwtk3W2SFV4+C9xoa5u7zl9  KXyK4xV7yt7hoB4AAAGhMAEdCAAABAMARzBFAiEAvYEdhWrpWzBamTzs/OhOhMIU  5XpSDVMzVxYMnVqzcncCIGLvcP9+EY2aXAsw9z8LjAjBcSAx701IiE7RdLkqKcRD  MBsGCSsGAQQBgjcVCgQOMAwwCgYIKwYBBQUHAwEwPAYJKwYBBAGCNxUHBC8wLQYl  KwYBBAGCNxUIh73XG4Hn60aCgZ0ujtAMh/DaHV2F9u8qgejiNQIBZAIBJzCCAQsG  CCsGAQUFBwEBBIH+MIH7MGEGCCsGAQUFBzAChlVodHRwOi8vd3d3Lm1pY3Jvc29m  dC5jb20vcGtpb3BzL2NlcnRzL01pY3Jvc29mdCUyMFRMUyUyMEcyJTIwUlNBJTIw  Q0ElMjBPQ1NQJTIwMDQuY3J0MGcGCCsGAQUFBzAChltodHRwOi8vY2Fpc3N1ZXJz  Lm1pY3Jvc29mdC5jb20vcGtpb3BzL2NlcnRzL01pY3Jvc29mdCUyMFRMUyUyMEcy  JTIwUlNBJTIwQ0ElMjBPQ1NQJTIwMDQuY3J0MC0GCCsGAQUFBzABhiFodHRwOi8v  b25lb2NzcC5taWNyb3NvZnQuY29tL29jc3AwHQYDVR0OBBYEFK+jg0J/hYh+sJ+d  l6RGu9iOOt8vMA4GA1UdDwEB/wQEAwIFoDAgBgNVHREEGTAXghVyZHAud3ZkLm1p  Y3Jvc29mdC5jb20wDAYDVR0TAQH/BAIwADCB8QYDVR0fBIHpMIHmMIHjoIHgoIHd  hmxodHRwOi8vd3d3Lm1pY3Jvc29mdC5jb20vcGtpb3BzL2NybC9wYXJ0aXRpb24v  TWljcm9zb2Z0JTIwVExTJTIwRzIlMjBSU0ElMjBDQSUyME9DU1AlMjAwNF9QYXJ0  aXRpb24wMDAzNi5jcmyGbWh0dHA6Ly9jcmwyLm1pY3Jvc29mdC5jb20vcGtpb3Bz  L2NybC9wYXJ0aXRpb24vTWljcm9zb2Z0JTIwVExTJTIwRzIlMjBSU0ElMjBDQSUy  ME9DU1AlMjAwNF9QYXJ0aXRpb24wMDAzNi5jcmwwZgYDVR0gBF8wXTAIBgZngQwB  AgIwUQYMKwYBBAGCN0yDfQEBMEEwPwYIKwYBBQUHAgEWM2h0dHA6Ly93d3cubWlj  cm9zb2Z0LmNvbS9wa2lvcHMvRG9jcy9SZXBvc2l0b3J5Lmh0bTAfBgNVHSMEGDAW  gBRUDLzsGPd98QPihL40ZERnz3UfZTATBgNVHSUEDDAKBggrBgEFBQcDATANBgkq  hkiG9w0BAQwFAAOCAgEAmXcjkcq0YgjdWlyed6lotqBSBgsQCfOzSCePgiP75rkR  6Vd0tEcmGpZ93PDeRwLBWH8cRnewy9WgcAGYni19htwVAr5hkcD6TOaiBTPvVQII  JlEqmbKJa/vdQhW+b5WDLW178stVl5kZADJCjRtJaAIr3vIMmdouiRc/HQV5JkVx  6gpZovaJz/juMYZ1ydk/QungEV2I4JnQ56gztq0w+seu68+ewiYR4P2XaWG2SeiW  Fkt37E5Q9jISo4xy7SomZUVlvXlAZu2cfTk3Dxk2OK4eJsQWQO2OIej9ZM0za/c7  6PXSDY2Rtje/qPwZDsz+FgkIV8WXxkmbXCczZM6HSXy22xAf/KbaKim84ZfrPG3Y  fKBGZteRjpCRTWB6LQ9HczEJ/LsMphUtylO6K0laFBvebSyO8C8ej9yxigYt//zm  a371qpeCisSh4aRsylOwggoMXwlwZASBf4mqlVPhVyH5LaLfcKW624kJPoa22ijh  dKuvQTX8ee6jvNyfHN6OmhyNJxIU7KyJX7JpvNR0hzhMMMlfHOfHJt4eao/1/Uks  lnuD6J8uqUtkA8+ORIwd5IXXsJYUjj+twq8EXFq55j2cUPmHWxOvFyM/NAWY4ofA  ayiZI8kG1OseXS13hyZdDjJJs6scwQStiNVUBjdrRklmXTeXeVVZ4wxmP44Bue8x  ggGVMIIBkQIBATBuMFcxCzAJBgNVBAYTAlVTMR4wHAYDVQQKExVNaWNyb3NvZnQg  Q29ycG9yYXRpb24xKDAmBgNVBAMTH01pY3Jvc29mdCBUTFMgRzIgUlNBIENBIE9D  U1AgMDQCE0MBQdqBpy6EZaFmd10AAAFB2oEwCwYJYIZIAWUDBAIBMAsGCSqGSIb3  DQEBAQSCAQAk0bNMbf7G26a4jcukJXfCUEqVZjlKpl/mef6GwRvlZY6c2g9Gv4t7  QvoGnw2LXKx77G3Y0P7INFPXGgU38U54/SxTJOzqzuhZClD0fqzDdCXuTmZnzE5E  M6KYCbla6ioYCgCFff0GDgYF3pq3k1AXqd1AMOu6ltmdJCSZ//SDWVyD6Ly8osgi  bR/wXLVlO53em7wIN5IN6d/r7RAq5YyXsxgK9wli4iy8iOP8+FKrXJRglo4UaZ2+  LUyzrsx2n76hVDHJxu6BI5+Qi6oIBK17Zmx//s7hrtebi8dhol9ao8LtNB5pNWQq  IAc4E17PRsLMJfvS45qjytohMq6VTn3l\r\n"
        );

        let verified = verify_rdp_signature(signed_rdp).unwrap();
        assert_eq!(verified.signed_scope.len(), 30);
        assert!(verified.signer_subject.contains("rdp.wvd.microsoft.com"));
        assert!(!verified.signer_certificate_der.is_empty());

        // Tampering with a signed property must fail RSA signature verification.
        let tampered = signed_rdp.replace(
            "full address:s:rdgateway-r1.wvd.microsoft.com",
            "full address:s:attacker.example.com",
        );
        assert!(matches!(
            verify_rdp_signature(&tampered),
            Err(Error::InvalidRdpSignature("RSA signature verification failed"))
        ));

        // Adding an unsigned sensitive setting must fail the sensitive-settings scope check.
        let unsigned_shell = format!("{signed_rdp}alternate shell:s:cmd.exe\r\n");
        assert!(matches!(
            verify_rdp_signature(&unsigned_shell),
            Err(Error::InvalidRdpSignature(
                "sensitive .rdp setting is not covered by signscope"
            ))
        ));
    }
}
