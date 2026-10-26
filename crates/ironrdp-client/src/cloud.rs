//! Cloud PC discovery and Windows App browser handoff, not native AVD reverse-connect.
//!
//! Authentication and browser launch belong to the caller.
//! Graph bearer tokens are not RDP session credentials.

use core::fmt;
use core::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue, USER_AGENT};
use serde::Deserialize;
use url::Url;

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

    /// Download an `.rdp` file from an AVD workspace resource URL.
    pub async fn download_avd_rdp_file(&self, rdp_url: Url, access_token: &str) -> Result<String, Error> {
        validate_avd_service_url(&rdp_url)?;
        let body = self
            .get_with_accept(
                rdp_url,
                access_token,
                "application/x-rdp, text/plain, */*",
                Some(AVD_FEED_USER_AGENT),
            )
            .await?;
        String::from_utf8(body).map_err(|_| Error::InvalidXml("RDP file is not valid UTF-8"))
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
}
