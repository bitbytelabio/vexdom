//! What a site publishes about its own pages (`sitemap.ts`): its robots.txt and the sitemaps that
//! names, read before a mapping run starts. Only the mapped origin is read. Its host is resolved
//! once and refused when that is a private address; every request then connects to that one
//! address (no second lookup to rebind), follows at most two redirects by hand and only within the
//! origin, and reads at most 4 MiB a document, expanded, and 16 MiB in all.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

use flate2::read::MultiGzDecoder;
use regex::Regex;
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, LOCATION};
use url::{Host, Url};
use vexdom_core::log;

use super::abort::Abort;
use vexdom_pure::text::utf16_prefix;

const MAX_DOCUMENTS: usize = 20;
const MAX_DOC_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_URLS: usize = 5_000;
const MAX_PATHS: usize = 200;
const MAX_PATTERNS: usize = 12;
const MAX_REDIRECTS: usize = 2;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_EVIDENCE_BYTES: usize = 512 * 1024;

/// `\s` in a JavaScript regex, as the inside of a character class.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// `/^\s*sitemap\s*:\s*(\S+)/i`; the letters spelled out, since JavaScript's `i` folds ASCII only.
static DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "^[{JS_SPACE}]*[Ss][Ii][Tt][Ee][Mm][Aa][Pp][{JS_SPACE}]*:[{JS_SPACE}]*([^{JS_SPACE}]+)"
    ))
    .expect("the sitemap directive pattern compiles")
});

/// `/<loc>\s*([^<\s]+)\s*<\/loc>/gi`.
static LOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "<[Ll][Oo][Cc]>[{JS_SPACE}]*([^<{JS_SPACE}]+)[{JS_SPACE}]*</[Ll][Oo][Cc]>"
    ))
    .expect("the loc pattern compiles")
});

/// `/<sitemapindex[\s>]/i`.
static SITEMAP_INDEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "<[Ss][Ii][Tt][Ee][Mm][Aa][Pp][Ii][Nn][Dd][Ee][Xx][{JS_SPACE}>]"
    ))
    .expect("the sitemap index pattern compiles")
});

/// One shape of URL a sitemap lists: how many pages share it, and one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlPattern {
    pub pattern: String,
    pub count: usize,
    pub example: String,
}

/// `SiteIndex`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteIndex {
    /// `robots`, `sitemap.xml`, `sitemap-index` or `none`.
    pub source: &'static str,
    pub documents: usize,
    pub url_count: usize,
    pub truncated: bool,
    pub paths: Vec<String>,
    pub patterns: Vec<UrlPattern>,
    /// The first documents as read, kept as the run's evidence.
    pub raw: String,
}

impl SiteIndex {
    /// No sitemap published (`EMPTY`).
    pub fn none() -> Self {
        Self {
            source: "none",
            documents: 0,
            url_count: 0,
            truncated: false,
            paths: Vec::new(),
            patterns: Vec::new(),
            raw: String::new(),
        }
    }
}

/// The mapped origin, its host resolved once (`Seed`).
#[derive(Debug)]
struct Seed {
    origin: String,
    scheme: String,
    /// Lowercase, as `Url::host_str` gives it (`[…]` around an IPv6 address).
    hostname: String,
    /// The port when the origin names one other than its scheme's.
    port: Option<u16>,
    address: IpAddr,
}

/// What a run has read so far: bytes across every document, and documents received.
#[derive(Debug, Default)]
struct Budget {
    bytes: usize,
    documents: usize,
}

/// `fetchSiteIndex(origin, signal)`: what `origin` says about its own pages; an empty index when
/// it says nothing or may not be read.
pub async fn fetch_site_index(origin: &str, signal: &Abort) -> SiteIndex {
    match pin_origin(origin).await {
        Ok(seed) => index_from(&seed, signal).await,
        Err(error) => {
            log::log(&format!("sitemap: refusing to fetch {origin}: {error}"));
            SiteIndex::none()
        }
    }
}

/// The rest of `fetchSiteIndex` once the origin is pinned: robots.txt, then the sitemaps it
/// declares (or the three usual places), a sitemap index among them followed to its sitemaps.
async fn index_from(seed: &Seed, signal: &Abort) -> SiteIndex {
    let client = match client_for(seed) {
        Ok(client) => client,
        Err(error) => {
            log::log(&format!(
                "sitemap: refusing to fetch {}: Error: {error}",
                seed.origin
            ));
            return SiteIndex::none();
        }
    };
    let mut budget = Budget::default();
    let mut evidence: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();
    let mut source = "none";
    let mut truncated = false;

    let robots_url = format!("{}/robots.txt", seed.origin);
    let robots = get(&client, &robots_url, seed, &mut budget, signal)
        .await
        .filter(|robots| !robots.is_empty());
    if let Some(robots) = &robots {
        evidence.push(format!("# {robots_url}\n{robots}"));
    }

    let declared = robots
        .as_deref()
        .map(sitemap_directives)
        .unwrap_or_default();
    let candidates = if declared.is_empty() {
        ["sitemap.xml", "sitemap_index.xml", "sitemap-index.xml"]
            .map(|name| format!("{}/{name}", seed.origin))
            .to_vec()
    } else {
        source = "robots";
        declared
    };

    let mut queue: VecDeque<String> = candidates.iter().cloned().collect();
    let mut seen: HashSet<String> = HashSet::new();
    while !signal.aborted() && budget.documents < MAX_DOCUMENTS && urls.len() < MAX_URLS {
        let Some(next) = queue.pop_front() else {
            break;
        };
        if !seen.insert(next.clone()) {
            continue;
        }
        let Some(body) = get(&client, &next, seed, &mut budget, signal)
            .await
            .filter(|body| !body.is_empty())
        else {
            continue;
        };
        if evidence.len() < 2 {
            evidence.push(format!("# {next}\n{body}"));
        }
        if source == "none" {
            source = "sitemap.xml";
        }

        // A document counts as an index only among the first ones read, as many as the run went
        // looking for, so the sitemaps an index names are read for their URLs.
        if SITEMAP_INDEX.is_match(&body) && seen.len() <= candidates.len() {
            source = "sitemap-index";
            for child in locs(&body) {
                if queue.len() + seen.len() >= MAX_DOCUMENTS {
                    break;
                }
                queue.push_back(child);
            }
            continue;
        }
        for loc in locs(&body) {
            if urls.len() >= MAX_URLS {
                truncated = true;
                break;
            }
            urls.push(loc);
        }
    }

    let mut paths = paths_of(&urls, seed);
    let patterns = patterns_of(&paths);
    let more = paths.len() > MAX_PATHS;
    paths.truncate(MAX_PATHS);
    SiteIndex {
        source: if urls.is_empty() { "none" } else { source },
        documents: budget.documents,
        url_count: urls.len(),
        truncated: truncated || more,
        paths,
        patterns,
        raw: utf16_prefix(&evidence.join("\n\n"), MAX_EVIDENCE_BYTES).to_owned(),
    }
}

/// `pinOrigin`: the origin with its host resolved, unless that is not http(s) or is private.
async fn pin_origin(origin: &str) -> Result<Seed, String> {
    let url = Url::parse(origin).map_err(|_| "TypeError: Invalid URL".to_owned())?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Error: unsupported scheme {scheme}:"));
    }
    let hostname = url.host_str().unwrap_or_default().to_lowercase();
    let address = resolve_once(&url).await?;
    if is_private_address(address) {
        return Err(format!(
            "Error: {hostname} resolves to the private address {address}"
        ));
    }
    Ok(Seed {
        origin: url.origin().ascii_serialization(),
        scheme: scheme.to_owned(),
        hostname,
        port: url.port(),
        address,
    })
}

/// `resolveOnce`: an address host as it is, a name through the system resolver (its first
/// answer).
async fn resolve_once(url: &Url) -> Result<IpAddr, String> {
    match url.host() {
        Some(Host::Ipv4(address)) => Ok(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => Ok(IpAddr::V6(address)),
        Some(Host::Domain(name)) => {
            let port = url.port_or_known_default().unwrap_or(0);
            let mut found = tokio::net::lookup_host((name, port))
                .await
                .map_err(|error| format!("Error: {error}"))?;
            found
                .next()
                .map(|address| address.ip())
                .ok_or_else(|| format!("Error: getaddrinfo ENOTFOUND {name}"))
        }
        None => Err("Error: no host to resolve".into()),
    }
}

/// One client per run: its host pinned to the address resolved for it (`pinnedLookup`), no
/// redirects followed for it, no proxy in between, nothing decompressed behind its back.
fn client_for(seed: &Seed) -> reqwest::Result<reqwest::Client> {
    vexdom_core::install_tls_provider();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        // Port 0: the URL's own port is used.
        .resolve(&seed.hostname, SocketAddr::new(seed.address, 0))
        .build()
}

/// `get`: one document of the mapped origin, following its redirects while they stay there.
async fn get(
    client: &reqwest::Client,
    target: &str,
    seed: &Seed,
    budget: &mut Budget,
    signal: &Abort,
) -> Option<String> {
    let mut url = Url::parse(target).ok()?;
    for hop in 0..=MAX_REDIRECTS {
        if !allowed(&url, seed) {
            log::log(&format!("sitemap: refused {url} (not the mapped origin)"));
            return None;
        }
        match send(client, &url, budget, signal).await? {
            Hop::Redirect(location) => {
                let location = location.filter(|location| !location.is_empty())?;
                if hop == MAX_REDIRECTS {
                    return None;
                }
                url = url.join(&location).ok()?;
            }
            Hop::Body(body) => return Some(decode(&body, &url)),
        }
    }
    None
}

/// `allowed`: the same scheme, host and port as the mapped origin.
fn allowed(url: &Url, seed: &Seed) -> bool {
    url.scheme() == seed.scheme
        && url
            .host_str()
            .is_some_and(|host| host.to_lowercase() == seed.hostname)
        && url.port() == seed.port
}

enum Hop {
    /// A 3xx, with its `Location` if it had one.
    Redirect(Option<String>),
    /// A 2xx body, cut short past the byte limits.
    Body(Vec<u8>),
}

/// `send`: one request, given up after ten seconds or when the run is cancelled.
async fn send(
    client: &reqwest::Client,
    url: &Url,
    budget: &mut Budget,
    signal: &Abort,
) -> Option<Hop> {
    let exchange = async {
        let mut response = client
            .get(url.clone())
            .header(ACCEPT, "text/plain,application/xml,text/xml,*/*")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            // Node reads header values as Latin-1.
            let location = response
                .headers()
                .get(LOCATION)
                .map(|value| value.as_bytes().iter().copied().map(char::from).collect());
            return Some(Hop::Redirect(location));
        }
        if !(200..300).contains(&status) {
            return None;
        }
        budget.documents += 1;
        Some(Hop::Body(read_capped(&mut response, budget).await))
    };
    tokio::select! {
        biased;
        () = signal.wait() => None,
        hop = tokio::time::timeout(REQUEST_TIMEOUT, exchange) => hop.ok().flatten(),
    }
}

/// `readCapped`: the body up to the chunk that would take it past 4 MiB, or the run past 16 MiB;
/// what arrived before a broken connection.
async fn read_capped(response: &mut reqwest::Response, budget: &mut Budget) -> Vec<u8> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        budget.bytes += chunk.len();
        if body.len() + chunk.len() > MAX_DOC_BYTES || budget.bytes > MAX_TOTAL_BYTES {
            break;
        }
        body.extend_from_slice(&chunk);
    }
    body
}

/// `decode`: UTF-8 text, expanded first when gzipped (by name or by its magic bytes); a gzip that
/// expands past 4 MiB, or does not expand at all, reads as nothing.
fn decode(bytes: &[u8], url: &Url) -> String {
    let gzipped = url.path().ends_with(".gz") || bytes.starts_with(&[0x1f, 0x8b]);
    if !gzipped {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut expanded = Vec::new();
    let read = MultiGzDecoder::new(bytes)
        .take(MAX_DOC_BYTES as u64 + 1)
        .read_to_end(&mut expanded);
    if read.is_err() || expanded.len() > MAX_DOC_BYTES {
        log::log(&format!(
            "sitemap: refused to expand {url} (over the decompressed size limit)"
        ));
        return String::new();
    }
    String::from_utf8_lossy(&expanded).into_owned()
}

/// `sitemapDirectives`: the `Sitemap:` lines of a robots.txt, at most twenty.
fn sitemap_directives(robots: &str) -> Vec<String> {
    robots
        .split('\n')
        .filter_map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            DIRECTIVE.captures(line).map(|found| found[1].to_owned())
        })
        .take(MAX_DOCUMENTS)
        .collect()
}

/// `locs`: every `<loc>` of a sitemap or sitemap index.
fn locs(xml: &str) -> Vec<String> {
    LOC.captures_iter(xml)
        .map(|found| decode_entities(&found[1]))
        .collect()
}

/// `decodeEntities`: the five XML entities, one after another as JavaScript replaces them.
fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// `pathsOf`: the distinct paths of the URLs on the mapped origin's scheme and host.
fn paths_of(urls: &[String], seed: &Seed) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for raw in urls {
        let Ok(url) = Url::parse(raw) else {
            continue;
        };
        if url.scheme() != seed.scheme
            || !url
                .host_str()
                .is_some_and(|host| host.to_lowercase() == seed.hostname)
        {
            continue;
        }
        let path = match url.path() {
            "" => "/",
            path => path,
        };
        if seen.insert(path.to_owned()) {
            paths.push(path.to_owned());
        }
    }
    paths
}

/// `patternsOf`: the shapes more than one path shares (its first segment kept, the rest an id, a
/// uuid or a slug), most common first.
fn patterns_of(paths: &[String]) -> Vec<UrlPattern> {
    let mut groups: Vec<UrlPattern> = Vec::new();
    let mut by_shape: HashMap<String, usize> = HashMap::new();
    for path in paths {
        let mut segments = path.split('/').filter(|segment| !segment.is_empty());
        let Some(first) = segments.next() else {
            continue;
        };
        let mut shape = format!("/{first}");
        for segment in segments {
            shape.push('/');
            shape.push_str(placeholder(segment));
        }
        match by_shape.get(&shape) {
            Some(&at) => groups[at].count += 1,
            None => {
                by_shape.insert(shape.clone(), groups.len());
                groups.push(UrlPattern {
                    pattern: shape,
                    count: 1,
                    example: path.clone(),
                });
            }
        }
    }
    groups.retain(|group| group.count > 1);
    groups.sort_by_key(|group| Reverse(group.count));
    groups.truncate(MAX_PATTERNS);
    groups
}

fn placeholder(segment: &str) -> &'static str {
    if segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return "{id}";
    }
    let bytes = segment.as_bytes();
    let uuid_like = bytes.len() >= 13
        && bytes[..8].iter().all(u8::is_ascii_hexdigit)
        && bytes[8] == b'-'
        && bytes[9..13].iter().all(u8::is_ascii_hexdigit);
    if uuid_like { "{uuid}" } else { "{slug}" }
}

/// `isPrivateAddress`: loopback, private, link-local, shared, benchmarking, multicast and the
/// like, also inside an IPv6 address that carries an IPv4 one (mapped, compatible, 6to4, NAT64).
/// Only an address that parsed reaches it; a host that does not resolve to one is never read.
fn is_private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, ..] = address.octets();
            a == 10
                || a == 127
                || a == 0
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (b == 18 || b == 19))
                || a >= 224
        }
        IpAddr::V6(address) => {
            let g = address.segments();
            if g.iter().all(|&part| part == 0)
                || (g[..7].iter().all(|&part| part == 0) && g[7] == 1)
            {
                return true;
            }
            if g[0] & 0xfe00 == 0xfc00
                || g[0] & 0xffc0 == 0xfe80
                || g[0] & 0xffc0 == 0xfec0
                || g[0] & 0xff00 == 0xff00
            {
                return true;
            }
            if g[0] == 0x2002 {
                return is_private_address(v4_from(g[1], g[2]));
            }
            if g[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_private_address(v4_from(g[6], g[7]));
            }
            if g[..5].iter().all(|&part| part == 0) && (g[5] == 0xffff || g[5] == 0) {
                return is_private_address(v4_from(g[6], g[7]));
            }
            false
        }
    }
}

fn v4_from(hi: u16, lo: u16) -> IpAddr {
    let [a, b] = hi.to_be_bytes();
    let [c, d] = lo.to_be_bytes();
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use axum::Router;
    use axum::http::{StatusCode, header};
    use axum::routing::get as route;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;
    use crate::agent::test_support::local;

    const PRIVATE: &[&str] = &[
        "10.0.0.1",
        "127.0.0.1",
        "127.1.2.3",
        "0.0.0.0",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.1",
        "192.168.1.1",
        "100.64.0.1",
        "192.0.0.1",
        "198.18.0.1",
        "224.0.0.1",
        "255.255.255.255",
        "::1",
        "::",
        "0:0:0:0:0:0:0:1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "fe90::1",
        "fea0::1",
        "febf::1",
        "::ffff:127.0.0.1",
        "::ffff:7f00:1",
        "0:0:0:0:0:ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "::ffff:a9fe:a9fe",
        "::127.0.0.1",
        "2002:7f00:1::",
        "64:ff9b::a9fe:a9fe",
        "fec0::1",
        "ff02::1",
        "",
        "not-an-ip",
        "1.2.3",
        "999.1.1.1",
    ];

    const PUBLIC: &[&str] = &[
        "8.8.8.8",
        "93.184.216.34",
        "1.1.1.1",
        "172.32.0.1",
        "192.169.1.1",
        "198.20.0.1",
        "223.255.255.255",
        "2606:4700::1111",
        "2001:4860:4860::8888",
        "::ffff:8.8.8.8",
        "2002:0808:0808::",
        "64:ff9b::808:808",
    ];

    /// What may not be connected to: a private address, or anything that is not an address.
    fn refused(address: &str) -> bool {
        address.parse().map_or(true, is_private_address)
    }

    fn seed(origin: &str, address: &str) -> Seed {
        let url = Url::parse(origin).unwrap();
        Seed {
            origin: url.origin().ascii_serialization(),
            scheme: url.scheme().to_owned(),
            hostname: url.host_str().unwrap().to_owned(),
            port: url.port(),
            address: address.parse().unwrap(),
        }
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Serves `app` on 127.0.0.1, with the seed a run would have pinned had it been allowed to
    /// read a private address (as `sitemap.test.ts` hands `get` one).
    async fn serve(app: impl FnOnce(&str) -> Router) -> Seed {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = app(&origin);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        seed(&origin, "127.0.0.1")
    }

    async fn fetch(seed: &Seed, target: &str) -> Option<String> {
        let client = client_for(seed).unwrap();
        get(&client, target, seed, &mut Budget::default(), &Abort::new()).await
    }

    #[test]
    fn private_addresses_are_refused_and_public_ones_are_not() {
        for address in PRIVATE {
            assert!(refused(address), "{address:?} is private");
        }
        for address in PUBLIC {
            assert!(!refused(address), "{address:?} is public");
        }
    }

    #[test]
    fn only_the_mapped_origin_is_allowed() {
        let seed = seed("https://example.com", "93.184.216.34");
        let cases = [
            ("https://example.com/sitemap.xml", true),
            ("https://EXAMPLE.com/a", true),
            ("http://example.com/a", false),
            ("https://evil.com/a", false),
            ("https://example.com:8443/a", false),
            ("https://169.254.169.254/latest/meta-data", false),
            ("https://sub.example.com/a", false),
        ];
        for (href, want) in cases {
            assert_eq!(allowed(&Url::parse(href).unwrap(), &seed), want, "{href}");
        }
    }

    fn live(_origin: &str) -> Router {
        Router::new()
            .route("/plain", route(|| async { "hello sitemap" }))
            .route(
                "/redirect",
                route(|| async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "http://169.254.169.254/")],
                    )
                }),
            )
            .route("/big", route(|| async { "x".repeat(5 * 1024 * 1024) }))
            .route("/gz", route(|| async { gzip(b"gzipped body") }))
    }

    #[test]
    fn a_live_server_is_read_within_its_limits() {
        local(async {
            let local = serve(live).await;
            let at = |path: &str| format!("{}{path}", local.origin);

            assert_eq!(
                fetch(&local, &at("/plain")).await.as_deref(),
                Some("hello sitemap")
            );
            assert_eq!(
                fetch(&local, &at("/redirect")).await,
                None,
                "a redirect to the metadata address is refused"
            );
            assert_eq!(
                fetch(&local, &at("/gz")).await.as_deref(),
                Some("gzipped body")
            );
            let big = fetch(&local, &at("/big"))
                .await
                .expect("an oversize body is cut short");
            assert!(big.len() <= MAX_DOC_BYTES);
            assert_eq!(
                fetch(&local, "http://169.254.169.254/").await,
                None,
                "another origin is refused"
            );
            assert_eq!(fetch(&local, &at("/missing")).await, None);
        });
    }

    #[test]
    fn a_gzip_that_expands_past_the_limit_reads_as_nothing() {
        let url = Url::parse("https://example.com/sitemap.xml.gz").unwrap();
        assert_eq!(decode(&gzip(&vec![b'x'; MAX_DOC_BYTES + 1]), &url), "");
        assert_eq!(
            decode(&gzip(&vec![b'x'; MAX_DOC_BYTES]), &url).len(),
            MAX_DOC_BYTES
        );
        assert_eq!(decode(b"not gzip", &url), "", "a .gz that is not one");
        let mut members = gzip(b"one ");
        members.extend(gzip(b"two"));
        assert_eq!(decode(&members, &url), "one two");
    }

    #[test]
    fn a_cancelled_run_reads_nothing_more() {
        local(async {
            let local = serve(live).await;
            let client = client_for(&local).unwrap();
            let signal = Abort::new();
            signal.abort();
            let read = get(
                &client,
                &format!("{}/plain", local.origin),
                &local,
                &mut Budget::default(),
                &signal,
            )
            .await;
            assert_eq!(read, None);
        });
    }

    #[test]
    fn robots_names_an_index_whose_sitemaps_are_read() {
        local(async {
            let local = serve(|origin| {
                let robots = format!(
                    "User-agent: *\r\nDisallow: /admin\r\n\u{FEFF}SITEMAP : {origin}/sitemap_index.xml\r\n"
                );
                let index = format!(
                    "<?xml version=\"1.0\"?>\n<sitemapindex xmlns=\"x\">\n<sitemap><loc>{origin}/posts.xml.gz</loc></sitemap>\n<sitemap><LOC> {origin}/pages.xml </LOC></sitemap>\n</sitemapindex>"
                );
                let posts = gzip(
                    format!(
                        "<urlset><url><loc>{origin}/blog/1</loc></url><url><loc>{origin}/blog/2</loc></url><url><loc>{origin}/blog/2#top</loc></url><url><loc>https://elsewhere.example/blog/9</loc></url></urlset>"
                    )
                    .as_bytes(),
                );
                let pages = format!(
                    "<urlset><url><loc>{origin}/pricing?a=1&amp;b=2</loc></url><url><loc>{origin}/blog/3</loc></url><url><loc>{origin}/team/0f8fad5b-d9cb-469f-a165-70867728950e</loc></url><url><loc>{origin}/team/ada</loc></url></urlset>"
                );
                Router::new()
                    .route("/robots.txt", route(move || async move { robots }))
                    .route("/sitemap_index.xml", route(move || async move { index }))
                    .route("/posts.xml.gz", route(move || async move { posts }))
                    .route("/pages.xml", route(move || async move { pages }))
            })
            .await;
            let index = index_from(&local, &Abort::new()).await;
            let origin = &local.origin;
            assert_eq!(index.source, "sitemap-index");
            assert_eq!(index.documents, 4);
            assert_eq!(index.url_count, 8);
            assert!(!index.truncated);
            assert_eq!(
                index.paths,
                [
                    "/blog/1",
                    "/blog/2",
                    "/pricing",
                    "/blog/3",
                    "/team/0f8fad5b-d9cb-469f-a165-70867728950e",
                    "/team/ada"
                ]
            );
            assert_eq!(
                index.patterns,
                [UrlPattern {
                    pattern: "/blog/{id}".into(),
                    count: 3,
                    example: "/blog/1".into()
                }]
            );
            assert!(
                index
                    .raw
                    .starts_with(&format!("# {origin}/robots.txt\nUser-agent: *\r\n"))
            );
            assert!(
                index
                    .raw
                    .contains(&format!("\n\n# {origin}/sitemap_index.xml\n<?xml"))
            );
            assert!(
                !index.raw.contains("posts.xml.gz\n"),
                "two documents at most"
            );
        });
    }

    #[test]
    fn without_robots_the_usual_places_are_tried() {
        local(async {
            let local = serve(|origin| {
                let urls = format!(
                    "<urlset><url><loc>{origin}/a</loc></url><url><loc>{origin}/b</loc></url></urlset>"
                );
                Router::new().route("/sitemap-index.xml", route(move || async move { urls }))
            })
            .await;
            let index = index_from(&local, &Abort::new()).await;
            assert_eq!(
                (index.source, index.documents, index.paths.clone()),
                ("sitemap.xml", 1, vec!["/a".to_owned(), "/b".to_owned()])
            );
            assert!(index.patterns.is_empty(), "no shape is shared");

            let silent = serve(|_| Router::new()).await;
            assert_eq!(index_from(&silent, &Abort::new()).await, SiteIndex::none());
        });
    }

    #[test]
    fn a_private_origin_is_not_read_at_all() {
        local(async {
            let local = serve(live).await;
            assert_eq!(
                fetch_site_index(&local.origin, &Abort::new()).await,
                SiteIndex::none()
            );
            assert!(pin_origin("ftp://example.com").await.is_err());
            assert_eq!(
                pin_origin("http://127.0.0.1:9/").await.err().as_deref(),
                Some("Error: 127.0.0.1 resolves to the private address 127.0.0.1")
            );
        });
    }

    #[test]
    fn robots_directives_and_locs_read_as_javascript_matched_them() {
        assert_eq!(
            sitemap_directives(
                "sitemap: https://a.example/1.xml\n  Sitemap:https://a.example/2.xml trailing\nNoSitemap: x\nsitemap:\n# Sitemap: https://a.example/3.xml"
            ),
            ["https://a.example/1.xml", "https://a.example/2.xml"]
        );
        assert_eq!(
            sitemap_directives(&"Sitemap: https://a.example/x.xml\n".repeat(30)).len(),
            MAX_DOCUMENTS
        );
        assert_eq!(
            locs(
                "<loc>https://a.example/?a=1&amp;lt;b</loc><loc>two words</loc><Loc>\n https://a.example/c \n</lOC>"
            ),
            ["https://a.example/?a=1<b", "https://a.example/c"]
        );
    }

    #[test]
    fn shapes_need_two_paths_and_sort_by_how_many_share_them() {
        let paths: Vec<String> = [
            "/",
            "/docs/intro",
            "/docs/setup",
            "/docs/api",
            "/blog/12",
            "/blog/13",
            "/u/0F8FAD5B-D9CB-x",
            "/u/0f8fad5b-d9cb-y",
            "/about",
        ]
        .map(str::to_owned)
        .to_vec();
        let shapes: Vec<(String, usize, String)> = patterns_of(&paths)
            .into_iter()
            .map(|pattern| (pattern.pattern, pattern.count, pattern.example))
            .collect();
        assert_eq!(
            shapes,
            [
                ("/docs/{slug}".to_owned(), 3, "/docs/intro".to_owned()),
                ("/blog/{id}".to_owned(), 2, "/blog/12".to_owned()),
                ("/u/{uuid}".to_owned(), 2, "/u/0F8FAD5B-D9CB-x".to_owned()),
            ]
        );
    }
}
