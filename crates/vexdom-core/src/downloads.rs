//! Captured downloads and saved screenshots (`src/daemon/{downloads,screenshots}.ts`,
//! `src/lib/downloads/limits.ts`). The daemon owns this end: it is the only half with a filesystem
//! and the only half that knows a run's scope. The agent gets notes, never bytes or a path to list.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::read_agent_config;
use crate::guardrails::scope::host_allowed;
use crate::{log, paths, rand};

pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_ATTACH_BYTES: u64 = 25 * 1024 * 1024;
pub const DOWNLOAD_TTL_DAYS: f64 = 14.0;

const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "app", "apk", "appimage", "bat", "cmd", "com", "deb", "dll", "dmg", "dylib", "exe", "gadget",
    "jar", "js", "jse", "ko", "ksh", "lnk", "msi", "msix", "mpkg", "out", "pkg", "ps1", "psm1",
    "reg", "rpm", "run", "scr", "sh", "so", "vb", "vbe", "vbs", "wsf", "wsh",
];

pub fn extension_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(dot) if dot > 0 => base[dot + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn is_executable_name(name: &str) -> bool {
    EXECUTABLE_EXTENSIONS.contains(&extension_of(name).as_str())
}

/// `describeSize`: `512 B`, `1.5 KB`, `100.0 MB`.
pub fn describe_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub notes: String,
    pub saved_to: String,
    pub captured_at: String,
}

/// What the extension reports after `page.captureDownload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedItem {
    pub browser_path: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub url: String,
    pub host: Option<String>,
}

/// `Err((code, message))` in `ActionResult` terms.
pub type Failure = (String, String);

fn fail<T>(code: &str, message: String) -> Result<T, Failure> {
    Err((code.to_owned(), message))
}

fn index_path() -> PathBuf {
    paths::state_dir().join("downloads.json")
}

pub fn download_dir() -> PathBuf {
    match read_agent_config().download_dir() {
        Some(dir) if !dir.trim().is_empty() => paths::expand_home(dir.trim()),
        _ => paths::user_dir().join("download"),
    }
}

fn read_index() -> Vec<DownloadRecord> {
    std::fs::read(index_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Written without a trailing newline, as the TS did.
fn write_index(records: &[DownloadRecord]) {
    let text = serde_json::to_string_pretty(records).unwrap_or_else(|_| "[]".into());
    if let Err(error) = paths::write_private(&index_path(), text.as_bytes()) {
        log::log_detail("could not write downloads.json", &error.to_string());
    }
}

fn discard(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// `yyyymmdd-hhmmss`, local time.
pub fn stamp() -> String {
    jiff::ZonedDateTime::now()
        .strftime("%Y%m%d-%H%M%S")
        .to_string()
}

/// A file name safe to write: no path, `[A-Za-z0-9._-]` only, no leading dots.
pub fn safe_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|base| base.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.trim_start_matches('.').to_owned()
}

/// Takes the file the browser wrote, or refuses it and deletes it.
pub fn adopt_download(
    item: &CapturedItem,
    hosts: Option<&[String]>,
) -> Result<DownloadRecord, Failure> {
    let browser_path = Path::new(&item.browser_path);
    if item.browser_path.is_empty() || !browser_path.exists() {
        return fail(
            "DOWNLOAD_MISSING",
            "The browser reported a download that is no longer on disk.".into(),
        );
    }
    if let (Some(hosts), Some(host)) = (hosts, item.host.as_deref())
        && !host_allowed(host, hosts)
    {
        discard(browser_path);
        return fail(
            "DOWNLOAD_OFF_SCOPE",
            format!(
                "That download came from {host}, which is not a site this run was asked about. It has been deleted."
            ),
        );
    }
    if is_executable_name(&item.name) {
        discard(browser_path);
        return fail(
            "DOWNLOAD_REFUSED",
            format!(
                "Vexdom does not keep executables — “{}” is a .{} file. It has been deleted.",
                item.name,
                extension_of(&item.name)
            ),
        );
    }
    let size = std::fs::metadata(browser_path).map_or(item.size, |meta| meta.len());
    if size > MAX_DOWNLOAD_BYTES {
        discard(browser_path);
        return fail(
            "DOWNLOAD_TOO_LARGE",
            format!(
                "That file is {}, over the {} download limit. It has been deleted.",
                describe_size(size),
                describe_size(MAX_DOWNLOAD_BYTES)
            ),
        );
    }
    sweep_downloads();
    let dir = download_dir();
    if let Err(error) = paths::ensure_private_dir(&dir) {
        return fail("DOWNLOAD_SAVE_FAILED", error.to_string());
    }
    let safe = safe_name(&item.name);
    let saved_to = dir.join(format!(
        "{}-{}",
        stamp(),
        if safe.is_empty() { "download" } else { &safe }
    ));
    if std::fs::rename(browser_path, &saved_to).is_err() {
        if let Err(error) = std::fs::copy(browser_path, &saved_to) {
            return fail("DOWNLOAD_SAVE_FAILED", error.to_string());
        }
        discard(browser_path);
    }
    #[cfg(unix)]
    let _ = std::fs::set_permissions(
        &saved_to,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    );
    let record = DownloadRecord {
        id: uuid_v4(),
        name: item.name.clone(),
        mime: item.mime.clone(),
        size,
        url: item.url.clone(),
        host: item.host.clone(),
        notes: notes_for(&saved_to, &item.name, &item.mime, size),
        saved_to: saved_to.to_string_lossy().into_owned(),
        captured_at: log::iso_now(),
    };
    let mut records = vec![record.clone()];
    records.extend(read_index());
    write_index(&records);
    log::log(&format!(
        "captured {} ({}) from {} → {}",
        item.name,
        describe_size(size),
        item.host.as_deref().unwrap_or("unknown host"),
        record.saved_to
    ));
    Ok(record)
}

/// `crypto.randomUUID()`.
pub fn uuid_v4() -> String {
    let mut bytes = rand::bytes(16);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `page.listDownloads`, answered by the daemon.
pub fn list_downloads(input: &Value) -> Value {
    sweep_downloads();
    let needle = input
        .get("nameContains")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let downloads: Vec<Value> = read_index()
        .into_iter()
        .filter(|record| Path::new(&record.saved_to).exists())
        .filter(|record| needle.as_deref().is_none_or(|needle| record.name.to_lowercase().contains(needle)))
        .map(|record| {
            let mut entry = json!({"id": record.id, "name": record.name, "mime": record.mime, "size": record.size});
            if let Some(host) = record.host {
                entry["host"] = json!(host);
            }
            entry["notes"] = json!(record.notes);
            entry["savedTo"] = json!(record.saved_to);
            entry["capturedAt"] = json!(record.captured_at);
            entry
        })
        .collect();
    json!({ "downloads": downloads })
}

fn read_download_bytes(id: &str) -> Result<(String, String, String), Failure> {
    let Some(record) = read_index().into_iter().find(|entry| entry.id == id) else {
        return fail(
            "DOWNLOAD_NOT_FOUND",
            format!(
                "No captured download with id \"{id}\". Call page.listDownloads to see what has been captured."
            ),
        );
    };
    if !Path::new(&record.saved_to).exists() {
        return fail(
            "DOWNLOAD_MISSING",
            format!(
                "“{}” is no longer in the download folder — capture it again.",
                record.name
            ),
        );
    }
    if record.size > MAX_ATTACH_BYTES {
        return fail(
            "DOWNLOAD_TOO_LARGE",
            format!(
                "“{}” is {}; files over {} cannot be attached to a page.",
                record.name,
                describe_size(record.size),
                describe_size(MAX_ATTACH_BYTES)
            ),
        );
    }
    let bytes = std::fs::read(&record.saved_to)
        .map_err(|error| ("DOWNLOAD_MISSING".to_owned(), error.to_string()))?;
    let mime = if record.mime.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        record.mime
    };
    Ok((record.name, mime, STANDARD.encode(bytes)))
}

/// The MIME type of a local file, by its extension.
fn mime_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("html") => "text/html",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// The most local files one `page.attachFile` takes.
const MAX_ATTACH_FILES: usize = 20;

/// `page.attachFile` with `paths`: local files on this computer, read here into `files`. The
/// Bridge's own state (its token, saved browser states) is never attached, and all the files
/// together stay under [`MAX_ATTACH_BYTES`].
fn read_local_files(paths: &[Value]) -> Result<Vec<Value>, Failure> {
    if paths.len() > MAX_ATTACH_FILES {
        return fail(
            "INVALID_INPUT",
            format!("Attach at most {MAX_ATTACH_FILES} files at once."),
        );
    }
    let state = std::fs::canonicalize(paths::state_dir()).unwrap_or_else(|_| paths::state_dir());
    let mut total = 0u64;
    paths
        .iter()
        .map(|path| {
            let shown = path.as_str().unwrap_or_default();
            let path = Path::new(shown);
            if !path.is_absolute() {
                return fail(
                    "INVALID_INPUT",
                    format!("“{shown}” is not an absolute path."),
                );
            }
            // Symlinks resolved, so a link cannot reach into the state folder either.
            let real = std::fs::canonicalize(path).ok();
            let meta = real
                .as_deref()
                .and_then(|real| std::fs::metadata(real).ok())
                .filter(|meta| meta.is_file());
            let (Some(real), Some(meta)) = (real, meta) else {
                return fail(
                    "FILE_NOT_FOUND",
                    format!("No file at “{shown}” on this computer."),
                );
            };
            if real.starts_with(&state) {
                return fail(
                    "INVALID_INPUT",
                    format!("“{shown}” is Vexdom's own state, which is never attached to a page."),
                );
            }
            total += meta.len();
            if meta.len() > MAX_ATTACH_BYTES || total > MAX_ATTACH_BYTES {
                return fail(
                    "FILE_TOO_LARGE",
                    format!(
                        "“{shown}” is {}; files over {} together cannot be attached to a page.",
                        describe_size(meta.len()),
                        describe_size(MAX_ATTACH_BYTES)
                    ),
                );
            }
            // Read no more than the size allows, should the file grow after it was measured.
            let mut bytes = Vec::new();
            std::fs::File::open(&real)
                .and_then(|file| file.take(MAX_ATTACH_BYTES).read_to_end(&mut bytes))
                .map_err(|error| ("FILE_NOT_FOUND".to_owned(), error.to_string()))?;
            let name = path.file_name().map_or_else(
                || "file".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            Ok(json!({"name": name, "mime": mime_for(path), "content": STANDARD.encode(bytes)}))
        })
        .collect()
}

/// `page.attachFile` with a `downloadId` or `paths`: the bytes come from this side. Caller-supplied
/// `name`, `mime`, `content` and `files` are always dropped.
pub fn resolve_attachment(action: &str, input: &Value) -> Result<Value, Failure> {
    if action != "page.attachFile" {
        return Ok(input.clone());
    }
    let mut args: Map<String, Value> = input.as_object().cloned().unwrap_or_default();
    for key in ["name", "mime", "content", "files"] {
        args.shift_remove(key);
    }
    let given = |args: &Map<String, Value>, key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
    };
    if let Some(paths) = args.shift_remove("paths") {
        if given(&args, "fileId") || given(&args, "downloadId") {
            return fail(
                "INVALID_INPUT",
                "Give \"paths\", \"fileId\" or \"downloadId\", only one of them.".into(),
            );
        }
        let Value::Array(paths) = paths else {
            return fail(
                "INVALID_INPUT",
                "\"paths\" is a list of absolute paths.".into(),
            );
        };
        if paths.is_empty() {
            return fail("INVALID_INPUT", "\"paths\" names no file.".into());
        }
        args.insert("files".into(), Value::Array(read_local_files(&paths)?));
        return Ok(Value::Object(args));
    }
    let Some(download_id) = args
        .get("downloadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    else {
        return Ok(Value::Object(args));
    };
    if args
        .get("fileId")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return fail(
            "INVALID_INPUT",
            "Give either \"fileId\" or \"downloadId\", not both.".into(),
        );
    }
    let (name, mime, content) = read_download_bytes(&download_id)?;
    args.insert("name".into(), json!(name));
    args.insert("mime".into(), json!(mime));
    args.insert("content".into(), json!(content));
    Ok(Value::Object(args))
}

fn ttl_days() -> f64 {
    read_agent_config()
        .download_ttl_days()
        .filter(|days| days.is_finite() && *days > 0.0)
        .unwrap_or(DOWNLOAD_TTL_DAYS)
}

/// Deletes downloads past their TTL; returns how many records went.
pub fn sweep_downloads() -> usize {
    let cutoff = log::now_ms() as f64 - ttl_days() * 24.0 * 60.0 * 60.0 * 1000.0;
    let records = read_index();
    let total = records.len();
    let keep: Vec<DownloadRecord> = records
        .into_iter()
        .filter(|record| {
            let path = Path::new(&record.saved_to);
            if !path.exists() {
                return false;
            }
            let at = record
                .captured_at
                .parse::<jiff::Timestamp>()
                .map_or(f64::NAN, |at| at.as_millisecond() as f64);
            if at >= cutoff {
                return true;
            }
            discard(path);
            false
        })
        .collect();
    if keep.len() != total {
        write_index(&keep);
    }
    total - keep.len()
}

pub fn clear_downloads() -> usize {
    let records = read_index();
    for record in &records {
        discard(Path::new(&record.saved_to));
    }
    write_index(&[]);
    let _ = std::fs::remove_dir_all(download_dir());
    records.len()
}

pub fn stored_downloads() -> Vec<DownloadRecord> {
    read_index()
        .into_iter()
        .filter(|record| Path::new(&record.saved_to).exists())
        .collect()
}

fn notes_for(path: &Path, name: &str, mime: &str, size: u64) -> String {
    let extension = extension_of(name);
    let kind = if mime.is_empty() {
        format!(
            "{} file",
            if extension.is_empty() {
                "unknown"
            } else {
                &extension
            }
        )
    } else {
        mime.to_owned()
    };
    let textual = mime.starts_with("text/")
        || [
            "application/json",
            "application/xml",
            "application/csv",
            "application/x-ndjson",
        ]
        .iter()
        .any(|prefix| mime.starts_with(prefix))
        || extension == "csv"
        || extension == "tsv";
    let shape = textual
        .then(|| text_shape(path, extension == "csv" || mime == "text/csv"))
        .flatten();
    match shape {
        Some(shape) => format!("{kind}, {} — {shape}", describe_size(size)),
        None => format!("{kind}, {}", describe_size(size)),
    }
}

fn text_shape(path: &Path, tabular: bool) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]).into_owned();
    let lines: Vec<&str> = head
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Some("empty".into());
    }
    if !tabular {
        return Some(format!("{} lines", lines.len()));
    }
    Some(format!(
        "{} rows × {} columns",
        lines.len(),
        lines[0].split(',').count()
    ))
}

/// Where uploaded skills live: `skillsDir` in config.json, else `~/vexdom/skills`.
pub fn uploaded_skills_dir() -> PathBuf {
    match read_agent_config().skills_dir() {
        Some(dir) if !dir.trim().is_empty() => paths::expand_home(dir.trim()),
        _ => paths::user_dir().join("skills"),
    }
}

pub fn screenshot_dir() -> PathBuf {
    match read_agent_config().screenshot_dir() {
        Some(dir) if !dir.trim().is_empty() => paths::expand_home(dir.trim()),
        _ => paths::user_dir().join("screenshot"),
    }
}

fn lexical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Writes screenshot bytes as a 0600 file; `dir` must lie under the uploaded skills folder.
pub fn save_screenshot(
    mime: &str,
    bytes: &[u8],
    filename: Option<&str>,
    dir: Option<&str>,
) -> Result<PathBuf, String> {
    let dir = match dir {
        Some(dir) => {
            let root = lexical(&uploaded_skills_dir());
            let target = lexical(Path::new(dir));
            if !target.starts_with(&root) {
                return Err(format!(
                    "refusing to write a screenshot outside {}",
                    root.display()
                ));
            }
            target
        }
        None => screenshot_dir(),
    };
    paths::ensure_private_dir(&dir).map_err(|error| error.to_string())?;
    let ext = match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };
    let name = match filename {
        Some(filename) => {
            let mut name = safe_name(filename);
            if name.is_empty() {
                name = format!("screenshot-{}", stamp());
            }
            if name.to_lowercase().ends_with(&format!(".{ext}")) {
                name
            } else {
                format!("{name}.{ext}")
            }
        }
        None => format!("screenshot-{}-{}.{ext}", stamp(), rand::hex(3)),
    };
    let target = dir.join(name);
    paths::write_private(&target, bytes).map_err(|error| error.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;

    #[test]
    fn limits() {
        assert_eq!(extension_of("a/b/Setup.EXE"), "exe");
        assert_eq!(extension_of(".bashrc"), "");
        assert!(is_executable_name("x.dmg"));
        assert_eq!(describe_size(512), "512 B");
        assert_eq!(describe_size(1536), "1.5 KB");
        assert_eq!(describe_size(MAX_DOWNLOAD_BYTES), "100.0 MB");
        assert_eq!(safe_name("../..hi there.csv"), "hi_there.csv");
        assert_eq!(uuid_v4().len(), 36);
    }

    #[test]
    fn adopt_list_attach() {
        let _home = scratch_home("downloads");
        let home = paths::state_dir();
        let dir = home.join("dl");
        std::fs::create_dir_all(&dir).unwrap();
        crate::paths::write_private_json(
            &crate::config::config_path(),
            &json!({"downloadDir": dir.join("kept").to_string_lossy()}),
        )
        .unwrap();
        let browser = dir.join("report.csv");
        std::fs::write(&browser, "a,b,c\n1,2,3\n").unwrap();
        let item = CapturedItem {
            browser_path: browser.to_string_lossy().into_owned(),
            name: "report.csv".into(),
            mime: "text/csv".into(),
            size: 0,
            url: "https://a.com/r.csv".into(),
            host: Some("a.com".into()),
        };
        let off = adopt_download(&item, Some(&["b.com".to_owned()])).unwrap_err();
        assert_eq!(off.0, "DOWNLOAD_OFF_SCOPE");
        assert!(!browser.exists());
        std::fs::write(&browser, "a,b,c\n1,2,3\n").unwrap();
        let record = adopt_download(&item, None).unwrap();
        assert_eq!(record.notes, "text/csv, 12 B — 2 rows × 3 columns");
        let listed = list_downloads(&json!({"nameContains": "REPORT"}));
        assert_eq!(listed["downloads"][0]["id"], record.id);
        let attached = resolve_attachment(
            "page.attachFile",
            &json!({"downloadId": record.id, "content": "forged"}),
        )
        .unwrap();
        assert_eq!(attached["content"], STANDARD.encode("a,b,c\n1,2,3\n"));
        assert_eq!(clear_downloads(), 1);
        assert!(stored_downloads().is_empty());
    }
}
