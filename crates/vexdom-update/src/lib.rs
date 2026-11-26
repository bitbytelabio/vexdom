//! Signed self-update (ADR-031): the latest GitHub release, downloads verified against the
//! release's `SHA256SUMS` and its minisign signature, and installing them over the copy that is
//! running: a release tarball's `bin/` and `extension/`, or the macOS `Vexdom.app` bundle.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const REPO: &str = "nguyenthdat/vexdom";
/// The release signing key's public half (`minisign -G`); the secret signs `SHA256SUMS` in CI.
pub const PUBLIC_KEY: &str = include_str!("../assets/minisign.pub");
/// The bundle identifier a staged `Vexdom.app` must carry.
pub const BUNDLE_ID: &str = "com.vexdom.desktop";

const SUMS: &str = "SHA256SUMS";
const SIGNATURE: &str = "SHA256SUMS.minisig";
const API_TIMEOUT: Duration = Duration::from_secs(8);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// A published release: its version (no `v`) and its assets as `(name, url)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub assets: Vec<(String, String)>,
}

impl Release {
    /// The release a GitHub `releases/latest` answer describes.
    pub fn from_github(answer: &Value) -> Result<Self, String> {
        let tag = answer
            .get("tag_name")
            .and_then(Value::as_str)
            .ok_or("The release has no tag")?;
        let assets = answer
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|asset| {
                Some((
                    asset.get("name")?.as_str()?.to_owned(),
                    asset.get("browser_download_url")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        Ok(Self {
            version: tag.trim_start_matches('v').to_owned(),
            assets,
        })
    }

    fn url_of(&self, name: &str) -> Result<&str, String> {
        self.assets
            .iter()
            .find(|(asset, _)| asset == name)
            .map(|(_, url)| url.as_str())
            .ok_or_else(|| {
                format!(
                    "UPDATE_ASSET_MISSING: release {} has no {name}",
                    self.version
                )
            })
    }
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("vexdom/", env!("CARGO_PKG_VERSION"), " update"))
        .timeout(timeout)
        .build()
        .map_err(|error| format!("UPDATE_UNREACHABLE: {error}"))
}

/// `GET https://api.github.com/repos/{REPO}/releases/latest`.
pub async fn latest() -> Result<Release, String> {
    let response = client(API_TIMEOUT)?
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("UPDATE_UNREACHABLE: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "UPDATE_UNREACHABLE: no release is published at https://github.com/{REPO}/releases"
        ));
    }
    let body = response
        .error_for_status()
        .map_err(|error| format!("UPDATE_UNREACHABLE: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("UPDATE_UNREACHABLE: {error}"))?;
    let answer: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("UPDATE_UNREACHABLE: GitHub's answer does not read: {error}"))?;
    Release::from_github(&answer)
}

/// `MAJOR.MINOR.PATCH` as numbers; anything after a `-` or `+` is ignored.
fn parts(version: &str) -> Option<[u64; 3]> {
    let core = version.trim_start_matches('v').split(['-', '+']).next()?;
    let mut numbers = core.split('.').map(|part| part.parse::<u64>().ok());
    let parsed = [numbers.next()??, numbers.next()??, numbers.next()??];
    numbers.next().is_none().then_some(parsed)
}

/// Whether `candidate` is a later version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    match (parts(candidate), parts(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let bytes = client(DOWNLOAD_TIMEOUT)?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("UPDATE_DOWNLOAD_FAILED: {url}: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("UPDATE_DOWNLOAD_FAILED: {url}: {error}"))?;
    Ok(bytes.to_vec())
}

/// Checks `sums` against its minisign `signature`, then `asset`'s line in it against `bytes`.
pub fn verify(
    sums: &[u8],
    signature: &str,
    asset: &str,
    bytes: &[u8],
    public_key: &str,
) -> Result<(), String> {
    let key = PublicKey::decode(public_key.trim())
        .map_err(|error| format!("SIGNATURE_INVALID: the release key does not read: {error}"))?;
    let signature = Signature::decode(signature)
        .map_err(|error| format!("SIGNATURE_INVALID: {SIGNATURE} does not read: {error}"))?;
    key.verify(sums, &signature, false).map_err(|_| {
        format!("SIGNATURE_INVALID: {SUMS} is not signed by the vexdom release key")
    })?;
    let sums =
        std::str::from_utf8(sums).map_err(|_| format!("SIGNATURE_INVALID: {SUMS} is not text"))?;
    let expected = sums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
        .map(|(hash, _)| hash.to_ascii_lowercase())
        .ok_or_else(|| format!("CHECKSUM_MISMATCH: {SUMS} does not list {asset}"))?;
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "CHECKSUM_MISMATCH: {asset} is not the file the release signed"
        ))
    }
}

/// Downloads `asset` into `dir` once `SHA256SUMS`, its signature and the asset's checksum agree.
pub async fn fetch_verified(release: &Release, asset: &str, dir: &Path) -> Result<PathBuf, String> {
    let sums = download(release.url_of(SUMS)?).await?;
    let signature = download(release.url_of(SIGNATURE)?).await?;
    let signature = String::from_utf8(signature)
        .map_err(|_| format!("SIGNATURE_INVALID: {SIGNATURE} is not text"))?;
    let bytes = download(release.url_of(asset)?).await?;
    verify(&sums, &signature, asset, &bytes, PUBLIC_KEY)?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join(asset);
    std::fs::write(&path, &bytes)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    Ok(path)
}

/// The release target this binary was built for.
pub fn target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

/// How the running copy was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// `<root>/bin/vexdom` beside `<root>/extension/chrome-mv3`, from a release tarball.
    Tarball { root: PathBuf },
    /// Inside `Vexdom.app/Contents/MacOS/`.
    App { bundle: PathBuf },
    /// Anything else: a build tree, a package manager.
    Unmanaged(PathBuf),
}

pub fn detect_layout(exe: &Path) -> Layout {
    let parent = |path: &Path| path.parent().map(Path::to_path_buf);
    let macos = parent(exe);
    if let Some(macos) = &macos
        && macos.ends_with("Contents/MacOS")
        && let Some(bundle) = macos.parent().and_then(Path::parent)
        && bundle
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        return Layout::App {
            bundle: bundle.to_path_buf(),
        };
    }
    if let Some(bin) = &macos
        && bin.file_name().is_some_and(|name| name == "bin")
        && let Some(root) = bin.parent()
        && root.join("extension/chrome-mv3/manifest.json").is_file()
    {
        return Layout::Tarball {
            root: root.to_path_buf(),
        };
    }
    Layout::Unmanaged(exe.to_path_buf())
}

/// A sibling of `path` named `<name><suffix>`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    path.with_file_name(format!("{name}{suffix}"))
}

/// `install.sh`'s `install_archive`: unpack beside `root`, check it holds `bin/vexdom` and the
/// extension, then replace `root`'s `bin/` and `extension/` (and LICENSE, NOTICE).
pub fn install_tarball(archive: &Path, root: &Path) -> Result<(), String> {
    let staging = sibling(root, ".new");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let result = unpack_and_swap(archive, root, &staging);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn unpack_and_swap(archive: &Path, root: &Path, staging: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    tar::Archive::new(flate2::read::GzDecoder::new(file))
        .unpack(staging)
        .map_err(|error| format!("UPDATE_ARCHIVE_INVALID: {error}"))?;
    let unpacked = std::fs::read_dir(staging)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .ok_or("UPDATE_ARCHIVE_INVALID: the archive is empty")?;
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    if !unpacked.join("bin/vexdom").is_file() {
        return Err(format!(
            "UPDATE_ARCHIVE_INVALID: {name} holds no bin/vexdom"
        ));
    }
    if !unpacked
        .join("extension/chrome-mv3/manifest.json")
        .is_file()
    {
        return Err(format!(
            "UPDATE_ARCHIVE_INVALID: {name} holds no extension/chrome-mv3"
        ));
    }
    for part in ["bin", "extension"] {
        let target = root.join(part);
        let old = sibling(&target, ".previous");
        let _ = std::fs::remove_dir_all(&old);
        if target.exists() {
            std::fs::rename(&target, &old).map_err(|error| error.to_string())?;
        }
        if let Err(error) = std::fs::rename(unpacked.join(part), &target) {
            let _ = std::fs::rename(&old, &target);
            return Err(format!(
                "Could not put {} in place: {error}",
                target.display()
            ));
        }
        let _ = std::fs::remove_dir_all(&old);
    }
    for file in ["LICENSE", "NOTICE"] {
        let from = unpacked.join(file);
        if from.is_file() {
            let _ = std::fs::rename(&from, root.join(file));
        }
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("{command:?}: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Unpacks an app zip beside `bundle` as `.Vexdom-<version>.app`, checks it is Vexdom at
/// `version`, and clears the quarantine flag. Returns the staged bundle.
pub fn stage_app(zip: &Path, bundle: &Path, version: &str) -> Result<PathBuf, String> {
    let dir = bundle.parent().ok_or("The app has no folder")?;
    let unpack = dir.join(format!(".Vexdom-{version}.unpack"));
    let staged = dir.join(format!(".Vexdom-{version}.app"));
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_dir_all(&staged);
    run(Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(&unpack))?;
    let app = unpack.join("Vexdom.app");
    let checked = check_bundle(&app, version)
        .and_then(|()| std::fs::rename(&app, &staged).map_err(|error| error.to_string()));
    let _ = std::fs::remove_dir_all(&unpack);
    checked?;
    let _ = run(Command::new("xattr")
        .arg("-dr")
        .arg("com.apple.quarantine")
        .arg(&staged));
    Ok(staged)
}

fn check_bundle(app: &Path, version: &str) -> Result<(), String> {
    let plist = app.join("Contents/Info.plist");
    let read = |key: &str| {
        run(Command::new("/usr/libexec/PlistBuddy")
            .arg("-c")
            .arg(format!("Print :{key}"))
            .arg(&plist))
    };
    let id = read("CFBundleIdentifier")?;
    let found = read("CFBundleShortVersionString")?;
    if id != BUNDLE_ID || found != version {
        return Err(format!(
            "UPDATE_ARCHIVE_INVALID: the download holds {id} {found}, not {BUNDLE_ID} {version}"
        ));
    }
    Ok(())
}

/// Swaps a staged app into place once process `$1` has quit, keeping the old one as
/// `<app>.previous` until the move succeeds, then opens it. `$1` pid, `$2` staged, `$3` destination.
pub fn relaunch_script() -> &'static str {
    r#"tries=0
while kill -0 "$1" 2>/dev/null && [ "$tries" -lt 150 ]; do sleep 0.2; tries=$((tries + 1)); done
if kill -0 "$1" 2>/dev/null; then rm -rf "$2"; exit 1; fi
rm -rf "$3.previous"
[ -e "$3" ] && mv "$3" "$3.previous"
if mv "$2" "$3"; then rm -rf "$3.previous"; else mv "$3.previous" "$3"; fi
open -n "$3""#
}

/// Moves a staged app into place now (no app running), keeping the old one until it succeeds.
pub fn swap_app(staged: &Path, bundle: &Path) -> Result<(), String> {
    let old = sibling(bundle, ".previous");
    let _ = std::fs::remove_dir_all(&old);
    if bundle.exists() {
        std::fs::rename(bundle, &old).map_err(|error| error.to_string())?;
    }
    if let Err(error) = std::fs::rename(staged, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(format!(
            "Could not put {} in place: {error}",
            bundle.display()
        ));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Hands the swap to a detached `sh` that waits for `pid` to quit.
pub fn swap_after_exit(pid: u32, staged: &Path, bundle: &Path) -> Result<(), String> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(relaunch_script())
        .arg("vexdom-update")
        .arg(pid.to_string())
        .arg(staged)
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.10.0", "0.9.2"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("0.9.2", "0.9.2"));
        assert!(!newer("0.9.1", "0.9.2"));
        assert!(!newer("garbage", "0.9.2"));
        assert!(newer("0.9.3-rc.1", "0.9.2"));
    }

    #[test]
    fn layouts() {
        let dir = std::env::temp_dir().join(format!("vexdom-update-{}", std::process::id()));
        let manifest = dir.join("root/extension/chrome-mv3/manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, "{}").unwrap();
        assert_eq!(
            detect_layout(&dir.join("root/bin/vexdom")),
            Layout::Tarball {
                root: dir.join("root")
            }
        );
        assert_eq!(
            detect_layout(Path::new("/Applications/Vexdom.app/Contents/MacOS/vexdom")),
            Layout::App {
                bundle: PathBuf::from("/Applications/Vexdom.app")
            }
        );
        let built = Path::new("/src/vexdom/target/release/vexdom");
        assert_eq!(detect_layout(built), Layout::Unmanaged(built.to_path_buf()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tarball(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive.append_data(&mut header, name, *body).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn a_tarball_replaces_bin_and_extension_or_leaves_them() {
        let dir = std::env::temp_dir().join(format!("vexdom-tarball-{}", std::process::id()));
        let root = dir.join("vexdom");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("extension/chrome-mv3")).unwrap();
        std::fs::write(root.join("bin/vexdom"), b"old").unwrap();
        std::fs::write(root.join("extension/chrome-mv3/manifest.json"), b"old").unwrap();
        std::fs::write(root.join("keep.txt"), b"mine").unwrap();

        let broken = dir.join("broken.tar.gz");
        tarball(&broken, &[("vexdom-1.0.0/bin/vexdom", b"new")]);
        assert!(
            install_tarball(&broken, &root)
                .unwrap_err()
                .contains("holds no extension/chrome-mv3")
        );
        assert_eq!(std::fs::read(root.join("bin/vexdom")).unwrap(), b"old");

        let good = dir.join("good.tar.gz");
        tarball(
            &good,
            &[
                ("vexdom-1.0.0/bin/vexdom", b"new"),
                ("vexdom-1.0.0/extension/chrome-mv3/manifest.json", b"{}"),
                ("vexdom-1.0.0/NOTICE", b"notice"),
            ],
        );
        install_tarball(&good, &root).unwrap();
        assert_eq!(std::fs::read(root.join("bin/vexdom")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(root.join("extension/chrome-mv3/manifest.json")).unwrap(),
            b"{}"
        );
        assert_eq!(std::fs::read(root.join("NOTICE")).unwrap(), b"notice");
        assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"mine");
        assert!(!sibling(&root, ".new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A signature made with the minisign CLI over `sums` (key below), then tampered with.
    #[test]
    fn signatures_and_checksums_are_both_checked() {
        let key = "untrusted comment: minisign public key 53504DEFC824D969\nRWRp2STI701QU4be0x05GwYqHItp8vy532xFyvgP8G5swoLAXIcJJBns\n";
        let asset = b"hello\n";
        let sums = include_bytes!("../tests/fixtures/SHA256SUMS");
        let signature = include_str!("../tests/fixtures/SHA256SUMS.minisig");
        assert_eq!(verify(sums, signature, "hello.txt", asset, key), Ok(()));
        assert!(
            verify(sums, signature, "hello.txt", b"tampered\n", key)
                .unwrap_err()
                .starts_with("CHECKSUM_MISMATCH")
        );
        assert!(
            verify(sums, signature, "other.txt", asset, key)
                .unwrap_err()
                .starts_with("CHECKSUM_MISMATCH")
        );
        let mut forged = sums.to_vec();
        forged[0] ^= 1;
        assert!(
            verify(&forged, signature, "hello.txt", asset, key)
                .unwrap_err()
                .starts_with("SIGNATURE_INVALID")
        );
        assert_eq!(key.trim(), PUBLIC_KEY.trim());
    }
}
