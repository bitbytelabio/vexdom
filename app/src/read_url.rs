//! `vexdom read <url>` and MCP `snapshot {"as":"markdown","url"}`: a page fetched over HTTP, never
//! touching a tab, as markdown. Ported from agent-browser (`cli/src/read.rs` at f0a9835,
//! Apache-2.0, see NOTICE): Accept negotiation, the `.md` probe, `llms.txt` links, `--llms`,
//! `--require-md`, `--raw`, a 2 MiB body cap. Every URL and redirect hop is checked against the
//! user's Blocked sites list in the browser first; HTML converts with the Bridge's converter.

use reqwest::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION, USER_AGENT};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::time::Duration;
use url::Url;
use vexdom_client::Bridge;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const BODY_LIMIT: usize = 2 * 1024 * 1024;
const READ_ACCEPT: &str = "text/markdown, text/plain;q=0.9, text/html;q=0.7, */*;q=0.1";
const USER_AGENT_VALUE: &str = concat!("vexdom/", env!("CARGO_PKG_VERSION"), " read");
/// Redirects followed by hand, each hop checked against the Blocked sites list.
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmsMode {
    Index,
    Full,
}

pub fn parse_llms_mode(raw: &str) -> Result<LlmsMode, String> {
    match raw {
        "index" => Ok(LlmsMode::Index),
        "full" => Ok(LlmsMode::Full),
        _ => Err(format!(
            "Invalid read --llms value '{}': expected index or full",
            raw
        )),
    }
}

#[derive(Debug, Clone)]
pub struct ReadOptions {
    /// Return the fetched response body without markdown or HTML extraction.
    pub raw: bool,
    /// Fail unless the selected response is served as Content-Type: text/markdown.
    pub require_md: bool,
    /// Return the nearest ancestor llms.txt or llms-full.txt view.
    pub llms: Option<LlmsMode>,
    /// Return a heading outline for the selected page content.
    pub outline: bool,
    /// Filter page sections, /llms.txt links, /llms-full.txt sections, or outline headings.
    pub filter: Option<String>,
    /// HTTP request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Extra HTTP headers. A supplied Accept header disables markdown negotiation fallbacks.
    pub headers: HashMap<String, String>,
    /// Whether loopback, private and link-local addresses may be fetched (the user's CLI only).
    pub allow_private: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            raw: false,
            require_md: false,
            llms: None,
            outline: false,
            filter: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            headers: HashMap::new(),
            allow_private: false,
        }
    }
}

pub fn parse_timeout_ms(raw: &str) -> Result<u64, String> {
    let ms = raw
        .parse::<u64>()
        .map_err(|_| format!("Invalid read timeout: {}", raw))?;
    if ms == 0 {
        return Err("Read timeout must be greater than 0".to_string());
    }
    Ok(ms)
}

pub fn options_from_command(cmd: &Value) -> Result<ReadOptions, String> {
    let timeout_ms = cmd
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    if timeout_ms == 0 {
        return Err("Read timeout must be greater than 0".to_string());
    }
    let llms = cmd
        .get("llms")
        .and_then(|v| v.as_str())
        .map(parse_llms_mode)
        .transpose()?;
    let mut headers = HashMap::new();
    if let Some(value) = cmd.get("headers") {
        let map = value
            .as_object()
            .ok_or_else(|| "read headers must be a JSON object".to_string())?;
        for (key, value) in map {
            if let Some(value) = value.as_str() {
                headers.insert(key.to_string(), value.to_string());
            }
        }
    }

    Ok(ReadOptions {
        raw: cmd.get("raw").and_then(|v| v.as_bool()).unwrap_or(false),
        require_md: cmd
            .get("requireMd")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        llms,
        outline: cmd
            .get("outline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        filter: cmd
            .get("filter")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        timeout_ms,
        headers,
        allow_private: false,
    })
}

pub fn normalize_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Read URL is empty".to_string());
    }

    let candidate = if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains("://")
    {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };

    let mut url = Url::parse(&candidate).map_err(|e| format!("Invalid read URL: {}", e))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "Unsupported read URL scheme '{}': use http or https",
                scheme
            ));
        }
    }
    if url.host_str().is_none() {
        return Err("Read URL must include a host".to_string());
    }
    url.set_fragment(None);
    Ok(url)
}

struct ReadFetch {
    final_url: String,
    status: u16,
    content_type: String,
    success: bool,
    body: String,
    truncated: bool,
}

#[derive(Clone)]
struct LlmsLink {
    title: String,
    url: Url,
}

/// An HTTP client that asks the browser about every URL it is about to fetch.
pub struct Fetcher<'a> {
    client: Client,
    bridge: &'a Bridge,
    allow_private: bool,
}

impl Fetcher<'_> {
    /// Before every fetch: the browser's Blocked sites list (`SITE_BLOCKED`, or `EXTENSION_OFFLINE`:
    /// the list lives in the browser) and the guardrails of a navigation to `url` (scope, URL
    /// payload), then, for an agent, no address on this computer or its network.
    async fn check(&self, url: &Url) -> Result<(), String> {
        let checked = self
            .bridge
            .invoke(
                "page.checkSites",
                &json!({"url": url.as_str(), "urls": [url.as_str()]}),
            )
            .await;
        checked
            .result
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        if !self.allow_private {
            refuse_private(url).await?;
        }
        Ok(())
    }
}

/// `READ_REFUSED` when `url`'s host is, or resolves to, a loopback, private, link-local or
/// otherwise local address.
async fn refuse_private(url: &Url) -> Result<(), String> {
    let refused = || {
        format!(
            "READ_REFUSED: {} is on this computer or its local network; an agent reads public pages only",
            url.host_str().unwrap_or("")
        )
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<std::net::IpAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![ip.into()],
        Some(url::Host::Ipv6(ip)) => vec![ip.into()],
        Some(url::Host::Domain(name)) => tokio::net::lookup_host((name, port))
            .await
            .map_err(|error| format!("Read request failed: {name}: {error}"))?
            .map(|address| address.ip())
            .collect(),
        None => return Err(refused()),
    };
    if addresses.is_empty() || addresses.iter().any(|ip| is_local(*ip)) {
        return Err(refused());
    }
    Ok(())
}

fn is_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64
        }
        std::net::IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_local(v4.into());
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

pub async fn run_read(
    bridge: &Bridge,
    raw_url: &str,
    options: ReadOptions,
) -> Result<Value, String> {
    let target = normalize_url(raw_url)?;
    let client = Fetcher {
        client: Client::builder()
            .timeout(Duration::from_millis(options.timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?,
        bridge,
        allow_private: options.allow_private,
    };
    client.check(&target).await?;

    match options.llms {
        Some(LlmsMode::Index) => return run_llms_index(&client, &target, &options).await,
        Some(LlmsMode::Full) => return run_llms_full(&client, &target, &options).await,
        None => {}
    }

    let primary = fetch_read_url(&client, target.clone(), &options)
        .await
        .map_err(|e| format!("Read request failed: {}", e))?;

    if primary.success && direct_primary_response_is_usable(&primary, &options) {
        let (source, content) = content_from_fetch(&primary, &options)?;
        return Ok(read_json_from_content(
            &target, &primary, source, content, &options,
        ));
    }

    if !options.raw && !is_markdown_content_type(&primary.content_type) && should_try_md(&options) {
        if let Some(md_url) = markdown_fallback_url(&target) {
            match fetch_read_url(&client, md_url.clone(), &options).await {
                Ok(md)
                    if md.success
                        && markdown_fallback_content_type_is_usable(&md.content_type, &options) =>
                {
                    return Ok(read_json_from_content(
                        &target,
                        &md,
                        "path-markdown",
                        md.body.clone(),
                        &options,
                    ));
                }
                Ok(_) | Err(_) => {}
            }
        }
    }

    if primary.success && !options.require_md && is_plain_text_content_type(&primary.content_type) {
        let (source, content) = content_from_fetch(&primary, &options)?;
        return Ok(read_json_from_content(
            &target, &primary, source, content, &options,
        ));
    }

    if !options.raw && should_try_md(&options) {
        if let Some(llms) = try_llms_link(&client, &target, &options).await {
            return llms;
        }
    }

    if !primary.success {
        return Err(format!("Read failed with HTTP {}", primary.status));
    }

    let (source, content) = content_from_fetch(&primary, &options)?;
    Ok(read_json_from_content(
        &target, &primary, source, content, &options,
    ))
}

async fn run_llms_index(
    client: &Fetcher<'_>,
    target: &Url,
    options: &ReadOptions,
) -> Result<Value, String> {
    let fetch = fetch_first_llms_file(client, target, "llms.txt", options).await?;
    let content = format_llms_index(&fetch.body, &fetch.final_url, options.filter.as_deref())?;
    Ok(read_json(target, &fetch, "llms-index", content))
}

async fn run_llms_full(
    client: &Fetcher<'_>,
    target: &Url,
    options: &ReadOptions,
) -> Result<Value, String> {
    let fetch = fetch_first_llms_file(client, target, "llms-full.txt", options).await?;
    let content = if let Some(filter) = options.filter.as_deref() {
        filter_markdown_sections(&fetch.body, filter, "No matching llms-full.txt sections")
    } else {
        fetch.body.clone()
    };
    Ok(read_json(target, &fetch, "llms-full", content))
}

async fn try_llms_link(
    client: &Fetcher<'_>,
    target: &Url,
    options: &ReadOptions,
) -> Option<Result<Value, String>> {
    let (llms_url, llms) = fetch_optional_llms_file(client, target, "llms.txt", options).await?;
    let link = find_llms_link_for_target(&llms.body, &llms_url, target)?;
    let fetch = fetch_read_url(client, link.url.clone(), options)
        .await
        .ok()?;
    if !fetch.success {
        return None;
    }
    if options.require_md && !is_markdown_content_type(&fetch.content_type) {
        return None;
    }
    let content = if is_html_content_type(&fetch.content_type) {
        vexdom_daemon::markdown::html_to_markdown(&fetch.body)
    } else {
        fetch.body.clone()
    };
    Some(Ok(read_json_from_content(
        target,
        &fetch,
        "llms-link",
        content,
        options,
    )))
}

async fn fetch_read_url(
    client: &Fetcher<'_>,
    target: Url,
    options: &ReadOptions,
) -> Result<ReadFetch, String> {
    let mut url = target;
    let mut hops = 0;
    let response = loop {
        client.check(&url).await?;
        let mut request = client
            .client
            .get(url.clone())
            .header(USER_AGENT, USER_AGENT_VALUE);
        let has_accept_header = options
            .headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("accept"));
        if !has_accept_header {
            request = request.header(ACCEPT, READ_ACCEPT);
        }
        for (key, value) in &options.headers {
            request = request.header(key, value);
        }
        let response = request.send().await.map_err(format_reqwest_error)?;
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .filter(|_| response.status().is_redirection());
        let Some(location) = location else {
            break response;
        };
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err("too many redirects".to_string());
        }
        url = url
            .join(location)
            .map_err(|e| format!("Invalid redirect: {}", e))?;
    };
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let success = response.status().is_success();
    let (body, truncated) = read_limited_text(response).await?;

    Ok(ReadFetch {
        final_url,
        status,
        content_type,
        success,
        body,
        truncated,
    })
}

fn content_from_fetch(
    fetch: &ReadFetch,
    options: &ReadOptions,
) -> Result<(&'static str, String), String> {
    let content_type_base = fetch.content_type.split(';').next().unwrap_or("").trim();
    let content_type_lower = content_type_base.to_ascii_lowercase();
    if options.require_md && !is_markdown_content_type(&fetch.content_type) {
        Err(expected_markdown_error(&fetch.content_type))
    } else if options.raw {
        Ok(("raw", fetch.body.clone()))
    } else if is_markdown_like_content_type(&fetch.content_type) {
        Ok(("accept-markdown", fetch.body.clone()))
    } else if content_type_lower == "text/plain" {
        Ok(("text", fetch.body.clone()))
    } else if content_type_lower == "text/html" || content_type_lower == "application/xhtml+xml" {
        Ok((
            "html-fallback",
            vexdom_daemon::markdown::html_to_markdown(&fetch.body),
        ))
    } else {
        Ok(("raw", fetch.body.clone()))
    }
}

fn format_reqwest_error(error: reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(err) = source {
        let part = err.to_string();
        if !part.is_empty() && !message.contains(&part) {
            message.push_str(": ");
            message.push_str(&part);
        }
        source = err.source();
    }
    message
}

fn read_json(target: &Url, fetch: &ReadFetch, source: &str, content: String) -> Value {
    json!({
        "url": target.to_string(),
        "finalUrl": fetch.final_url.clone(),
        "status": fetch.status,
        "contentType": fetch.content_type.clone(),
        "source": source,
        "truncated": fetch.truncated,
        "content": content,
    })
}

fn read_json_from_content(
    target: &Url,
    fetch: &ReadFetch,
    source: &str,
    content: String,
    options: &ReadOptions,
) -> Value {
    if options.outline {
        let outline = format_page_outline(&content, &fetch.final_url, options.filter.as_deref());
        read_json(target, fetch, &format!("{}-outline", source), outline)
    } else if let Some(filter) = options.filter.as_deref() {
        let filtered = filter_page_sections(&content, filter);
        read_json(target, fetch, &format!("{}-filtered", source), filtered)
    } else {
        read_json(target, fetch, source, content)
    }
}

fn is_markdown_content_type(content_type: &str) -> bool {
    content_type_base(content_type).eq_ignore_ascii_case("text/markdown")
}

fn is_markdown_like_content_type(content_type: &str) -> bool {
    matches!(
        content_type_base(content_type).as_str(),
        "text/markdown" | "text/x-markdown" | "application/markdown"
    )
}

fn is_plain_text_content_type(content_type: &str) -> bool {
    content_type_base(content_type) == "text/plain"
}

fn content_type_base(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn direct_primary_response_is_usable(fetch: &ReadFetch, options: &ReadOptions) -> bool {
    options.raw
        || is_markdown_content_type(&fetch.content_type)
        || (!options.require_md && is_markdown_like_content_type(&fetch.content_type))
}

fn markdown_fallback_content_type_is_usable(content_type: &str, options: &ReadOptions) -> bool {
    if options.require_md {
        is_markdown_content_type(content_type)
    } else {
        is_markdown_like_content_type(content_type) || is_plain_text_content_type(content_type)
    }
}

fn expected_markdown_error(content_type: &str) -> String {
    let base = content_type.split(';').next().unwrap_or("").trim();
    format!(
        "Expected text/markdown, got {}",
        if base.is_empty() {
            "unknown content type"
        } else {
            base
        }
    )
}

fn is_html_content_type(content_type: &str) -> bool {
    let base = content_type_base(content_type);
    base == "text/html" || base == "application/xhtml+xml"
}

fn should_try_md(options: &ReadOptions) -> bool {
    !options
        .headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("accept"))
}

fn markdown_fallback_url(url: &Url) -> Option<Url> {
    if url.path().ends_with(".md") {
        return None;
    }
    let mut md_url = url.clone();
    let path = url.path();
    let next_path = if path == "/" || path.is_empty() {
        "/index.md".to_string()
    } else {
        format!("{}.md", path.trim_end_matches('/'))
    };
    md_url.set_path(&next_path);
    Some(md_url)
}

async fn fetch_first_llms_file(
    client: &Fetcher<'_>,
    target: &Url,
    filename: &str,
    options: &ReadOptions,
) -> Result<ReadFetch, String> {
    let mut last_status = None;
    for url in llms_file_candidates(target, filename) {
        let fetch = fetch_read_url(client, url, options)
            .await
            .map_err(|e| format!("Read request failed: {}", e))?;
        if fetch.success {
            if is_html_content_type(&fetch.content_type) {
                last_status = Some(fetch.status);
                continue;
            }
            if options.require_md && !is_markdown_content_type(&fetch.content_type) {
                return Err(expected_markdown_error(&fetch.content_type));
            }
            return Ok(fetch);
        }
        last_status = Some(fetch.status);
    }

    match last_status {
        Some(status) => Err(format!("{} failed with HTTP {}", filename, status)),
        None => Err(format!("{} not found", filename)),
    }
}

async fn fetch_optional_llms_file(
    client: &Fetcher<'_>,
    target: &Url,
    filename: &str,
    options: &ReadOptions,
) -> Option<(Url, ReadFetch)> {
    for url in llms_file_candidates(target, filename) {
        let fetch = fetch_read_url(client, url.clone(), options).await.ok()?;
        if fetch.success && !is_html_content_type(&fetch.content_type) {
            return Some((url, fetch));
        }
    }
    None
}

fn llms_file_candidates(url: &Url, filename: &str) -> Vec<Url> {
    let mut candidates = Vec::new();
    let mut prefixes = Vec::new();
    let path = url.path().trim_matches('/');
    if !path.is_empty() {
        let segments = path.split('/').collect::<Vec<_>>();
        for len in (1..=segments.len()).rev() {
            prefixes.push(format!("/{}", segments[..len].join("/")));
        }
    }
    prefixes.push(String::new());

    for prefix in prefixes {
        let mut candidate = url.clone();
        let path = if prefix.is_empty() {
            format!("/{}", filename)
        } else {
            format!("{}/{}", prefix.trim_end_matches('/'), filename)
        };
        candidate.set_path(&path);
        candidate.set_query(None);
        candidate.set_fragment(None);
        if !candidates
            .iter()
            .any(|existing: &Url| existing == &candidate)
        {
            candidates.push(candidate);
        }
    }

    candidates
}

fn parse_llms_links(body: &str, base_url: &Url) -> Vec<LlmsLink> {
    let mut links = Vec::new();
    for line in body.lines() {
        let Some(line) = markdown_list_item_text(line) else {
            continue;
        };
        let mut cursor = 0;
        while let Some(label_start_rel) = line[cursor..].find('[') {
            let label_start = cursor + label_start_rel;
            if label_start > 0 && line.as_bytes().get(label_start - 1) == Some(&b'!') {
                cursor = label_start + 1;
                continue;
            }
            let Some(label_end_rel) = line[label_start + 1..].find("](") else {
                break;
            };
            let label_end = label_start + 1 + label_end_rel;
            let href_start = label_end + 2;
            let Some(href_end_rel) = line[href_start..].find(')') else {
                break;
            };
            let href_end = href_start + href_end_rel;
            let title = line[label_start + 1..label_end].trim();
            let href = line[href_start..href_end]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('<')
                .trim_matches('>');
            if !title.is_empty() && !href.is_empty() {
                if let Ok(url) = base_url.join(href) {
                    links.push(LlmsLink {
                        title: title.to_string(),
                        url,
                    });
                }
            }
            cursor = href_end + 1;
        }
    }
    links
}

fn find_llms_link_for_target(body: &str, base_url: &Url, target: &Url) -> Option<LlmsLink> {
    let target_key = doc_match_key(target)?;
    let links = dedupe_llms_links(parse_llms_links(body, base_url));
    if let Some(exact) = links
        .iter()
        .find(|link| doc_match_key(&link.url).as_ref() == Some(&target_key))
    {
        return Some(exact.clone());
    }

    let target_origin = origin_key(target)?;
    let target_segment = last_doc_segment(target)?;
    let mut candidates = links
        .into_iter()
        .filter(|link| origin_key(&link.url).as_ref() == Some(&target_origin))
        .filter(|link| {
            last_doc_segment(&link.url).as_ref() == Some(&target_segment)
                || slugify_label(&link.title) == target_segment
        })
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn markdown_list_item_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest);
        }
    }

    let marker_end = trimmed.find(['.', ')'])?;
    if marker_end == 0 || !trimmed[..marker_end].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    trimmed[marker_end + 1..].strip_prefix(' ')
}

fn dedupe_llms_links(mut links: Vec<LlmsLink>) -> Vec<LlmsLink> {
    let mut seen = HashSet::new();
    links.retain(|link| {
        seen.insert(format!(
            "{}\0{}",
            link.title.to_ascii_lowercase(),
            link.url.as_str()
        ))
    });
    links
}

fn doc_match_key(url: &Url) -> Option<String> {
    Some(format!("{}{}", origin_key(url)?, normalized_doc_path(url)))
}

fn origin_key(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let authority = if let Some(port) = url.port() {
        format!("{}:{}", host, port)
    } else {
        host.to_string()
    };
    Some(format!("{}://{}", url.scheme(), authority))
}

fn normalized_doc_path(url: &Url) -> String {
    let mut path = url.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path = "/".to_string();
    }
    if let Some(stripped) = path.strip_suffix(".md") {
        path = stripped.to_string();
    }
    if path.ends_with("/index") {
        path.truncate(path.len() - "/index".len());
        if path.is_empty() {
            path = "/".to_string();
        }
    }
    path
}

fn last_doc_segment(url: &Url) -> Option<String> {
    normalized_doc_path(url)
        .trim_matches('/')
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
}

fn slugify_label(label: &str) -> String {
    let mut slug = String::new();
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn format_llms_index(body: &str, final_url: &str, filter: Option<&str>) -> Result<String, String> {
    let base = Url::parse(final_url).map_err(|e| format!("Invalid llms.txt URL: {}", e))?;
    let mut links = dedupe_llms_links(parse_llms_links(body, &base));
    if let Some(filter) = filter {
        let needle = filter.to_ascii_lowercase();
        links.retain(|link| {
            link.title.to_ascii_lowercase().contains(&needle)
                || link.url.as_str().to_ascii_lowercase().contains(&needle)
        });
    }
    if links.is_empty() {
        if filter.is_some() {
            return Ok("No matching llms.txt links".to_string());
        }
        return Ok(normalize_markdownish(body));
    }

    let mut out = format!("# llms.txt\n\nSource: {}\n", final_url);
    for link in links {
        out.push_str(&format!("\n- [{}]({})", link.title, link.url));
    }
    Ok(out)
}

struct Heading {
    level: usize,
    title: String,
}

fn format_page_outline(content: &str, final_url: &str, filter: Option<&str>) -> String {
    let mut headings = parse_markdown_headings(content);
    if let Some(filter) = filter {
        let needle = filter.to_ascii_lowercase();
        headings.retain(|heading| heading.title.to_ascii_lowercase().contains(&needle));
    }
    if headings.is_empty() {
        if filter.is_some() {
            return "No matching headings".to_string();
        }
        return "No headings found".to_string();
    }

    let mut out = format!("# Outline\n\nSource: {}\n", final_url);
    for heading in headings {
        out.push('\n');
        out.push_str(&"  ".repeat(heading.level.saturating_sub(1)));
        out.push_str("- ");
        out.push_str(&heading.title);
    }
    out
}

fn parse_markdown_headings(content: &str) -> Vec<Heading> {
    content.lines().filter_map(parse_markdown_heading).collect()
}

fn filter_page_sections(content: &str, filter: &str) -> String {
    let needle = filter.to_ascii_lowercase();
    let lines = content.lines().collect::<Vec<_>>();
    let headings = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| parse_markdown_heading(line).map(|heading| (index, heading)))
        .collect::<Vec<_>>();

    let mut sections = Vec::new();
    let mut captured_until = 0;
    for (i, (start, heading)) in headings.iter().enumerate() {
        if *start < captured_until || !heading.title.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        let end = headings[i + 1..]
            .iter()
            .find(|(_, next)| next.level <= heading.level)
            .map(|(index, _)| *index)
            .unwrap_or(lines.len());
        captured_until = end;
        sections.push(lines[*start..end].join("\n").trim().to_string());
    }

    if !sections.is_empty() {
        return sections.join("\n\n");
    }

    filter_markdown_sections(content, filter, "No matching page sections")
}

fn parse_markdown_heading(line: &str) -> Option<Heading> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty()
        && !rest
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(false)
    {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim();
    if title.is_empty() {
        None
    } else {
        Some(Heading {
            level,
            title: title.to_string(),
        })
    }
}

fn filter_markdown_sections(body: &str, filter: &str, no_match_message: &str) -> String {
    let needle = filter.to_ascii_lowercase();
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in body.lines() {
        if line.trim_start().starts_with('#') && !current.trim().is_empty() {
            if current.to_ascii_lowercase().contains(&needle) {
                sections.push(current.trim().to_string());
            }
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.trim().is_empty() && current.to_ascii_lowercase().contains(&needle) {
        sections.push(current.trim().to_string());
    }

    if !sections.is_empty() {
        return sections.join("\n\n");
    }

    let matching_lines = body
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains(&needle))
        .collect::<Vec<_>>();
    if matching_lines.is_empty() {
        no_match_message.to_string()
    } else {
        matching_lines.join("\n")
    }
}

async fn read_limited_text(response: reqwest::Response) -> Result<(String, bool), String> {
    let mut response = response;
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        let remaining = BODY_LIMIT.saturating_sub(bytes.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((String::from_utf8_lossy(&bytes).to_string(), truncated))
}

fn normalize_markdownish(input: &str) -> String {
    let mut lines = Vec::new();
    let mut blank_count = 0;
    let mut in_fence = false;

    for raw in input.lines() {
        let line = if in_fence {
            raw.trim_end().to_string()
        } else {
            raw.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        if line.trim() == "```" {
            in_fence = !in_fence;
        }

        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blank_count = 0;
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}

/// A read's text for an agent: the content, credentials sealed.
pub fn sealed_content(read: &Value) -> String {
    vexdom_core::guardrails::secrets::seal_secrets(
        read.get("content").and_then(Value::as_str).unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_recognised() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_local(local.parse().unwrap()), "{local}");
        }
        for public in ["93.184.216.34", "2606:2800:220:1::1"] {
            assert!(!is_local(public.parse().unwrap()), "{public}");
        }
    }
}
