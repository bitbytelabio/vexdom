//! Saved logins: `bridge.authSave`, `bridge.authLogin`, `bridge.authList`, `bridge.authShow` and
//! `bridge.authDelete`, ported from agent-browser's `cli/src/native/auth.rs` and
//! `handle_auth_login` (`cli/src/native/actions.rs`), f0a9835, Apache-2.0, see NOTICE.
//!
//! Each login is one AES-256-GCM file in `~/.vexdom/vault/`, in agent-browser's envelope
//! (`{version, encrypted, iv, authTag, data}`), under the key in `VEXDOM_VAULT_KEY` (64 hex digits)
//! or `~/.vexdom/.vault-key`, which the first save creates. A login runs here, on one pinned tab:
//! its password goes from the vault, or from a `credential.read` plugin, straight into the page's
//! `page.fillInput`, and no result or error carries it.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead as _, KeyInit as _, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;
use vexdom_core::plugins::{self, CredentialResolveRequest, ResolvedCredential};
use vexdom_core::result::{ActionError, ActionResult, failure};
use vexdom_core::{log, paths, rand};

use crate::agent::InvokeOpts;
use crate::link::{Link, Reply};
use crate::{Daemon, unknown_action};

const KEY_ENV: &str = "VEXDOM_VAULT_KEY";
const KEY_FILE_NAME: &str = ".vault-key";
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// agent-browser's default action timeout.
const DEFAULT_TIMEOUT_MS: u64 = 25_000;
/// Poll interval used while waiting for auth form selectors to appear.
const SELECTOR_POLL: Duration = Duration::from_millis(100);
/// Time spent trying targeted username selectors before broad text-input fallback selectors are
/// allowed.
const PREFERRED_SELECTOR_WINDOW_MS: u64 = 5_000;
/// How long a submit may take to start a navigation, and the settle time when none comes.
const POST_SUBMIT_WINDOW_MS: u64 = 10_000;
const POST_SUBMIT_FALLBACK_MS: u64 = 2_000;

const OFFLINE: &str = "The Vexdom extension is not connected — open your browser with the extension loaded, then retry";
const NO_NAVIGATE_PAGE_ERROR: &str = "auth login --no-navigate requires an existing active HTTP(S) browser page; open the login page first";
const PAGE_CHANGED_ERROR: &str =
    "Active browser page changed during auth login --no-navigate; retry on the intended login page";
/// What stands in for a password wherever one would show.
const HIDDEN: &str = "********";

const PREFERRED_USER_SELECTORS: [&str; 16] = [
    "input[type=email]",
    "input[name=email]",
    "input[id=email]",
    "input[autocomplete=email]",
    "input[autocomplete=username]",
    "input[name=username]",
    "input[name*=email i]",
    "input[name*=user i]",
    "input[id*=email i]",
    "input[id*=user i]",
    "input[type=text][name*=email i]",
    "input[type=text][name*=user i]",
    "input[type=text][id*=email i]",
    "input[type=text][id*=user i]",
    "input[type=text][autocomplete=email]",
    "input[type=text][autocomplete=username]",
];
const FALLBACK_USER_SELECTORS: [&str; 2] = ["input[type=text]", "input:not([type])"];
const AUTO_SUBMIT_SELECTORS: [&str; 3] = [
    "button[type=submit]",
    "input[type=submit]",
    "button:not([type])",
];
const PASSWORD_SELECTOR: &str = "input[type=password]";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthProfile {
    name: String,
    url: String,
    username: String,
    password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submit_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_login_at: Option<String>,
}

/// The password stays out of any debug print.
impl std::fmt::Debug for AuthProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthProfile")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &HIDDEN)
            .field("username_selector", &self.username_selector)
            .field("password_selector", &self.password_selector)
            .field("submit_selector", &self.submit_selector)
            .finish_non_exhaustive()
    }
}

fn error(code: &str, message: impl Into<String>) -> ActionError {
    ActionError {
        code: code.to_owned(),
        message: message.into(),
    }
}

fn failed(message: impl Into<String>) -> ActionError {
    error("ACTION_FAILED", message)
}

fn key_missing(message: impl Into<String>) -> ActionError {
    error("VAULT_KEY_MISSING", message)
}

fn not_found(name: &str) -> ActionError {
    error(
        "AUTH_PROFILE_NOT_FOUND",
        format!("Auth profile '{}' not found", name),
    )
}

fn validate_profile_name(name: &str) -> Result<(), ActionError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(error(
            "INVALID_INPUT",
            format!(
                "Invalid profile name '{}'. Must match /^[a-zA-Z0-9_-]+$/",
                name
            ),
        ));
    }
    Ok(())
}

fn parse_key_hex(hex_str: &str) -> Option<[u8; 32]> {
    let hex_str = hex_str.trim();
    if hex_str.len() != 64 || !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// A profile in the JSON + base64 envelope agent-browser (and its Node.js predecessor) writes.
fn encrypt_profile(key: &[u8; 32], profile: &AuthProfile) -> Result<String, ActionError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| failed(format!("Encryption key error: {}", e)))?;

    let plaintext = serde_json::to_string(profile)
        .map_err(|e| failed(format!("Failed to serialize profile: {}", e)))?;

    let iv = rand::bytes(IV_LEN);
    let nonce =
        Nonce::<Aes256Gcm>::try_from(iv.as_slice()).map_err(|_| failed("Failed to generate IV"))?;

    // aes-gcm appends the 16-byte auth tag to the ciphertext
    let encrypted = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| failed(format!("Encryption failed: {}", e)))?;
    let (ciphertext, auth_tag) = encrypted.split_at(encrypted.len().saturating_sub(TAG_LEN));

    let payload = json!({
        "version": 1,
        "encrypted": true,
        "iv": STANDARD.encode(&iv),
        "authTag": STANDARD.encode(auth_tag),
        "data": STANDARD.encode(ciphertext),
    });

    serde_json::to_string_pretty(&payload)
        .map_err(|e| failed(format!("Failed to serialize payload: {}", e)))
}

/// The envelope's fields that decryption reads.
#[derive(Deserialize)]
struct EncryptedPayload {
    iv: String,
    #[serde(rename = "authTag")]
    auth_tag: String,
    data: String,
}

/// Unlike agent-browser, a plain unencrypted profile is not accepted: Vexdom never wrote one.
fn decrypt_profile(key: &[u8; 32], data: &[u8]) -> Result<AuthProfile, ActionError> {
    let payload: EncryptedPayload = serde_json::from_slice(data)
        .map_err(|_| failed("Profile is not a valid encrypted payload"))?;

    let iv = STANDARD
        .decode(&payload.iv)
        .map_err(|e| failed(format!("Invalid base64 iv: {}", e)))?;
    let auth_tag = STANDARD
        .decode(&payload.auth_tag)
        .map_err(|e| failed(format!("Invalid base64 authTag: {}", e)))?;
    let ciphertext = STANDARD
        .decode(&payload.data)
        .map_err(|e| failed(format!("Invalid base64 data: {}", e)))?;
    let nonce = Nonce::<Aes256Gcm>::try_from(iv.as_slice())
        .map_err(|_| failed(format!("Invalid iv: expected {IV_LEN} bytes")))?;

    // aes-gcm expects ciphertext || auth_tag as input to decrypt
    let mut combined = Vec::with_capacity(ciphertext.len() + auth_tag.len());
    combined.extend_from_slice(&ciphertext);
    combined.extend_from_slice(&auth_tag);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| failed(format!("Decryption key error: {}", e)))?;
    let plaintext = cipher
        .decrypt(&nonce, combined.as_slice())
        .map_err(|e| failed(format!("Decryption failed: {}", e)))?;

    let json_str = String::from_utf8(plaintext)
        .map_err(|e| failed(format!("Decrypted data is not valid UTF-8: {}", e)))?;
    serde_json::from_str(&json_str).map_err(|e| failed(format!("Invalid profile data: {}", e)))
}

/// Where profiles and their key live; the environment's in use, a scratch folder's in tests.
struct Vault {
    dir: PathBuf,
    key_file: PathBuf,
    env_key: Option<String>,
}

impl Vault {
    fn from_env() -> Self {
        let state = paths::state_dir();
        Self {
            dir: state.join("vault"),
            key_file: state.join(KEY_FILE_NAME),
            env_key: std::env::var_os(KEY_ENV).map(|key| key.to_string_lossy().into_owned()),
        }
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.json", name))
    }

    /// The key from `VEXDOM_VAULT_KEY`, else from the key file.
    fn key(&self) -> Result<[u8; 32], ActionError> {
        if let Some(key_hex) = &self.env_key {
            return parse_key_hex(key_hex).ok_or_else(|| {
                key_missing(format!(
                    "{} should be a 64-character hex string (256 bits). Generate one with: openssl rand -hex 32",
                    KEY_ENV
                ))
            });
        }

        if self.key_file.exists() {
            let hex = std::fs::read_to_string(&self.key_file)
                .map_err(|e| key_missing(format!("Failed to read encryption key file: {}", e)))?;
            return parse_key_hex(&hex).ok_or_else(|| {
                key_missing(format!(
                    "Invalid encryption key in {}. Expected 64-character hex string.",
                    self.key_file.display()
                ))
            });
        }

        Err(key_missing(format!(
            "Encryption key required. Set {} or ensure {} exists.",
            KEY_ENV,
            self.key_file.display()
        )))
    }

    /// The key, generated into the key file (0600) only when neither the variable nor the file
    /// exists; `true` when it was.
    fn ensure_key(&self) -> Result<([u8; 32], bool), ActionError> {
        if self.env_key.is_some() || self.key_file.exists() {
            return self.key().map(|key| (key, false));
        }
        let key: [u8; 32] = rand::bytes(32)
            .try_into()
            .map_err(|_| failed("Failed to generate key"))?;
        let key_hex = key.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        paths::write_private(&self.key_file, format!("{}\n", key_hex).as_bytes())
            .map_err(|e| failed(format!("Failed to write encryption key: {}", e)))?;
        Ok((key, true))
    }

    /// Writes the profile (0600, in a 0700 folder); `true` when the key was created for it.
    fn save(&self, profile: &AuthProfile) -> Result<bool, ActionError> {
        let (key, created) = self.ensure_key()?;
        let encrypted_json = encrypt_profile(&key, profile)?;
        paths::write_private(&self.profile_path(&profile.name), encrypted_json.as_bytes())
            .map_err(|e| failed(format!("Failed to write profile: {}", e)))?;
        Ok(created)
    }

    fn load(&self, name: &str) -> Result<AuthProfile, ActionError> {
        validate_profile_name(name)?;
        let path = self.profile_path(name);
        if !path.exists() {
            return Err(not_found(name));
        }
        let data =
            std::fs::read(&path).map_err(|e| failed(format!("Failed to read profile: {}", e)))?;
        decrypt_profile(&self.key()?, &data)
    }

    fn delete(&self, name: &str) -> ActionResult {
        validate_profile_name(name)?;
        let path = self.profile_path(name);
        if !path.exists() {
            return Err(not_found(name));
        }
        std::fs::remove_file(&path)
            .map_err(|e| failed(format!("Failed to delete profile: {}", e)))?;
        Ok(json!({ "deleted": name }))
    }

    /// Every profile's name, username and URL, by name; one that cannot be opened says so.
    fn list(&self) -> Value {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return json!({ "profiles": [] });
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
            .filter_map(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        let key = self.key().ok();
        let profiles: Vec<Value> = names
            .into_iter()
            .map(|name| {
                let profile = key.as_ref().and_then(|key| {
                    let data = std::fs::read(self.profile_path(&name)).ok()?;
                    decrypt_profile(key, &data).ok()
                });
                match profile {
                    Some(profile) => json!({
                        "name": profile.name,
                        "username": profile.username,
                        "url": profile.url,
                    }),
                    None => json!({
                        "name": name,
                        "error": "Failed to decrypt",
                    }),
                }
            })
            .collect();
        json!({ "profiles": profiles })
    }
}

fn required<'a>(input: &'a Value, key: &str) -> Result<&'a str, ActionError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| error("INVALID_INPUT", format!("Missing '{}'", key)))
}

/// A string field, an empty one counting as absent.
fn optional(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn save(vault: &Vault, input: &Value) -> ActionResult {
    let name = required(input, "name")?;
    let url = required(input, "url")?;
    let username = required(input, "username")?;
    let password = required(input, "password")?;
    validate_profile_name(name)?;
    let profile = AuthProfile {
        name: name.to_string(),
        url: url.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        username_selector: optional(input, "usernameSelector"),
        password_selector: optional(input, "passwordSelector"),
        submit_selector: optional(input, "submitSelector"),
        created_at: None,
        last_login_at: None,
    };
    let key_created = vault.save(&profile)?;
    let mut saved = json!({ "saved": name });
    if key_created {
        let notice = format!(
            "Auto-generated encryption key at {} -- back up this file or set {}",
            vault.key_file.display(),
            KEY_ENV
        );
        log::log(&notice);
        saved["keyCreated"] = json!(true);
        saved["notice"] = json!(notice);
    }
    Ok(saved)
}

fn show(vault: &Vault, input: &Value) -> ActionResult {
    let profile = vault.load(required(input, "name")?)?;
    Ok(json!({
        "profile": {
            "name": profile.name,
            "url": profile.url,
            "username": profile.username,
            "usernameSelector": profile.username_selector,
            "passwordSelector": profile.password_selector,
            "submitSelector": profile.submit_selector,
        }
    }))
}

/// The five `bridge.auth*` actions; only `bridge.authLogin` needs the browser.
pub(crate) async fn handle(
    daemon: &Daemon,
    link: Option<Rc<Link>>,
    action: &str,
    input: Value,
    opts: &InvokeOpts,
) -> Reply {
    // A recording running while the password is typed must not keep it.
    let secret = |password: &str| crate::flow::add_secret(daemon, password);
    // An external caller's `tabId` names the tab the login happens in.
    let opts = InvokeOpts {
        tab_id: opts
            .tab_id
            .or_else(|| input.get("tabId").and_then(Value::as_i64)),
        ..opts.clone()
    };
    run(&Vault::from_env(), link, action, &input, &opts, &secret)
        .await
        .into()
}

/// An auth action's input as a log line or the side panel may show it: `bridge.authSave`'s
/// password replaced.
pub(crate) fn shown(action: &str, input: &Value) -> Value {
    let mut shown = input.clone();
    if action == "bridge.authSave"
        && let Some(password) = shown.get_mut("password")
        && password.is_string()
    {
        *password = json!(HIDDEN);
    }
    shown
}

async fn run(
    vault: &Vault,
    link: Option<Rc<Link>>,
    action: &str,
    input: &Value,
    opts: &InvokeOpts,
    secret: &dyn Fn(&str),
) -> ActionResult {
    match action {
        "bridge.authSave" => save(vault, input),
        "bridge.authList" => Ok(vault.list()),
        "bridge.authShow" => show(vault, input),
        "bridge.authDelete" => vault.delete(required(input, "name")?),
        "bridge.authLogin" => {
            let request = LoginRequest::parse(input)?;
            let Some(link) = link.filter(|link| link.is_open()) else {
                return failure("EXTENSION_OFFLINE", OFFLINE);
            };
            let page = TabPage::pin(link, opts).await?;
            login(&page, &request, vault, secret).await
        }
        _ => unknown_action(action),
    }
}

/// `bridge.authLogin`'s input.
struct LoginRequest {
    name: String,
    no_navigate: bool,
    url: Option<String>,
    provider: Option<String>,
    item: Option<String>,
    username_selector: Option<String>,
    password_selector: Option<String>,
    submit_selector: Option<String>,
    timeout_ms: u64,
}

impl LoginRequest {
    fn parse(input: &Value) -> Result<Self, ActionError> {
        let provider = optional(input, "credentialProvider");
        let item = optional(input, "item");
        // A plugin's item can stand in for the profile name it is asked about.
        let name = match (input.get("name").and_then(Value::as_str), &provider, &item) {
            (Some(name), _, _) => name.to_owned(),
            (None, Some(_), Some(item)) => item.clone(),
            _ => return Err(error("INVALID_INPUT", "Missing 'name'")),
        };
        Ok(Self {
            name,
            no_navigate: input
                .get("noNavigate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            url: optional(input, "url"),
            provider,
            item,
            username_selector: optional(input, "usernameSelector"),
            password_selector: optional(input, "passwordSelector"),
            submit_selector: optional(input, "submitSelector"),
            timeout_ms: input
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS),
        })
    }
}

/// One page action on the login's tab.
trait Page {
    async fn call(&self, action: &str, input: Value) -> ActionResult;
}

/// The login's tab, the same for every step: the caller's, else the one active when the login
/// starts, so a tab switched to midway never receives the password.
struct TabPage {
    link: Rc<Link>,
    tab: Option<i64>,
    run_id: Option<String>,
}

impl TabPage {
    async fn pin(link: Rc<Link>, opts: &InvokeOpts) -> Result<Self, ActionError> {
        let tab = match opts.tab_id {
            Some(tab) => Some(tab),
            None => link
                .invoke("page.switchTab", &json!({}), None, opts.run_id.as_deref())
                .await
                .result?
                .get("activeTabId")
                .and_then(Value::as_i64),
        };
        Ok(Self {
            link,
            tab,
            run_id: opts.run_id.clone(),
        })
    }
}

impl Page for TabPage {
    async fn call(&self, action: &str, input: Value) -> ActionResult {
        self.link
            .invoke(action, &input, self.tab, self.run_id.as_deref())
            .await
            .result
    }
}

async fn url_of(page: &impl Page) -> Result<String, ActionError> {
    page.call("page.query", json!({ "what": "url" }))
        .await?
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| failed("The page did not say its URL"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthLoginOrigin(String);

/// Parse a URL into the normalized HTTP(S) origin used to constrain
/// `auth login --no-navigate`. Paths, queries, and fragments are intentionally
/// excluded, and URL serialization normalizes explicit default ports.
fn auth_login_origin(raw_url: &str, label: &str) -> Result<AuthLoginOrigin, String> {
    let has_unambiguous_authority = raw_url
        .split_once(':')
        .map(|(_, rest)| rest)
        .is_some_and(|rest| rest.starts_with("//") && !rest.starts_with("///"));
    let parsed = url::Url::parse(raw_url).map_err(|_| {
        format!(
            "Invalid {} URL: expected an absolute HTTP(S) URL with a host",
            label
        )
    })?;
    if !has_unambiguous_authority
        || !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
    {
        return Err(format!(
            "Invalid {} URL: expected an absolute HTTP(S) URL with a host",
            label
        ));
    }

    Ok(AuthLoginOrigin(parsed.origin().ascii_serialization()))
}

fn ensure_auth_login_origin_matches(
    current_origin: &AuthLoginOrigin,
    expected_origin: &AuthLoginOrigin,
) -> Result<(), String> {
    if current_origin == expected_origin {
        return Ok(());
    }
    Err(format!(
        "Current page origin {} does not match credential origin {}",
        current_origin.0, expected_origin.0
    ))
}

/// The tab's document is still on the credential's origin. Diagnostics contain origins only,
/// never URL paths or credential values.
async fn validate_auth_login_page(
    page: &impl Page,
    expected_origin: &AuthLoginOrigin,
) -> Result<(), ActionError> {
    let current_url = url_of(page)
        .await
        .map_err(|_| error("TAB_UNREACHABLE", PAGE_CHANGED_ERROR))?;
    let current_origin = auth_login_origin(&current_url, "current page")
        .map_err(|message| error("INVALID_TARGET", message))?;
    ensure_auth_login_origin_matches(&current_origin, expected_origin)
        .map_err(|message| error("INVALID_TARGET", message))
}

async fn check_origin(
    page: &impl Page,
    expected_origin: Option<&AuthLoginOrigin>,
) -> Result<(), ActionError> {
    match expected_origin {
        Some(origin) => validate_auth_login_page(page, origin).await,
        None => Ok(()),
    }
}

/// A `url` given with a login may only point at the credential's own origin: whoever asks for a
/// login never chooses where its password is typed.
fn same_site(asked: &str, own: &str) -> Result<(), ActionError> {
    let asked =
        auth_login_origin(asked, "login").map_err(|message| error("INVALID_INPUT", message))?;
    let own =
        auth_login_origin(own, "credential").map_err(|message| error("INVALID_INPUT", message))?;
    ensure_auth_login_origin_matches(&asked, &own)
        .map_err(|message| error("INVALID_TARGET", message))
}

/// The login itself: from the vault by name, or from the `credentialProvider` plugin.
async fn credential(vault: &Vault, request: &LoginRequest) -> Result<AuthProfile, ActionError> {
    let Some(provider) = &request.provider else {
        let mut profile = vault.load(&request.name)?;
        if let Some(url) = &request.url {
            same_site(url, &profile.url)?;
            profile.url.clone_from(url);
        }
        return Ok(profile);
    };
    let resolved = from_plugin(
        provider.clone(),
        request.name.clone(),
        request.item.clone(),
        request.url.clone(),
    )
    .await?;
    if let (Some(url), Some(own)) = (&request.url, &resolved.url) {
        same_site(url, own)?;
    }
    Ok(AuthProfile {
        name: request.name.clone(),
        url: request.url.clone().or(resolved.url).unwrap_or_default(),
        username: resolved.username,
        password: resolved.password,
        username_selector: resolved.username_selector,
        password_selector: resolved.password_selector,
        submit_selector: resolved.submit_selector,
        created_at: None,
        last_login_at: None,
    })
}

/// The plugin runs off the event loop; a failure is the plugin module's generic text.
async fn from_plugin(
    provider: String,
    name: String,
    item: Option<String>,
    url: Option<String>,
) -> Result<ResolvedCredential, ActionError> {
    let (configured, warnings) = plugins::configured();
    for warning in &warnings {
        log::log(warning);
    }
    let resolved = tokio::task::spawn_blocking(move || {
        plugins::resolve_credential_with_plugins(
            &provider,
            &configured,
            CredentialResolveRequest {
                profile_name: &name,
                item_ref: item.as_deref(),
                url: url.as_deref(),
            },
        )
    })
    .await;
    match resolved {
        Ok(Ok(credential)) => Ok(credential),
        Ok(Err(message)) => Err(error("PLUGIN_FAILED", message)),
        Err(_) => Err(error(
            "PLUGIN_FAILED",
            "The credential plugin stopped unexpectedly",
        )),
    }
}

/// `secret` hears the password before it is typed.
async fn login(
    page: &impl Page,
    request: &LoginRequest,
    vault: &Vault,
    secret: &dyn Fn(&str),
) -> ActionResult {
    // Bind no-navigation login to an HTTP(S) page before any credential is read.
    if request.no_navigate {
        let current_url = url_of(page)
            .await
            .map_err(|_| error("INVALID_TARGET", NO_NAVIGATE_PAGE_ERROR))?;
        auth_login_origin(&current_url, "current page")
            .map_err(|_| error("INVALID_TARGET", NO_NAVIGATE_PAGE_ERROR))?;
        if let Some(url) = &request.url {
            let override_origin = auth_login_origin(url, "credential")
                .map_err(|message| error("INVALID_INPUT", message))?;
            validate_auth_login_page(page, &override_origin).await?;
        }
    }
    let cred = credential(vault, request).await?;
    if cred.url.is_empty() {
        return Err(error("INVALID_INPUT", "Credential has no URL"));
    }
    let AuthProfile {
        url,
        username,
        password,
        username_selector: stored_username_selector,
        password_selector: stored_password_selector,
        submit_selector: stored_submit_selector,
        ..
    } = cred;
    secret(&password);

    let auth_timeout_ms = request.timeout_ms;
    let expected_origin = if request.no_navigate {
        let origin = auth_login_origin(&url, "credential")
            .map_err(|message| error("INVALID_INPUT", message))?;
        validate_auth_login_page(page, &origin).await?;
        Some(origin)
    } else {
        page.call("page.navigate", json!({ "url": url })).await?;
        // Pinned to where the login page landed: a navigation while the fields are awaited
        // must not carry the password to another site.
        let landed = url_of(page)
            .await
            .map_err(|_| error("TAB_UNREACHABLE", PAGE_CHANGED_ERROR))?;
        Some(
            auth_login_origin(&landed, "login page")
                .map_err(|message| error("INVALID_TARGET", message))?,
        )
    };

    let username_sel = request
        .username_selector
        .clone()
        .or(stored_username_selector);
    let password_sel = request
        .password_selector
        .clone()
        .or(stored_password_selector);
    let submit_sel = request.submit_selector.clone().or(stored_submit_selector);

    // Find and fill username
    let user_selector = match username_sel {
        Some(s) => wait_for_any_selector(page, &[s.as_str()], auth_timeout_ms)
            .await?
            .ok_or_else(|| {
                error(
                    "TIMEOUT",
                    format!("Timed out waiting for username selector '{}'", s),
                )
            })?,
        None => {
            let preferred_window_ms = auth_timeout_ms.min(PREFERRED_SELECTOR_WINDOW_MS);
            let fallback_window_ms = auth_timeout_ms.saturating_sub(preferred_window_ms);
            match wait_for_any_selector(page, &PREFERRED_USER_SELECTORS, preferred_window_ms)
                .await?
            {
                Some(selector) => selector,
                None => {
                    if fallback_window_ms == 0 {
                        return Err(error(
                            "TIMEOUT",
                            format!(
                                "Timed out waiting for username field (preferred selectors for {}ms: {})",
                                preferred_window_ms,
                                PREFERRED_USER_SELECTORS.join(", ")
                            ),
                        ));
                    }
                    wait_for_any_selector(page, &FALLBACK_USER_SELECTORS, fallback_window_ms)
                        .await?
                        .ok_or_else(|| {
                            error(
                                "TIMEOUT",
                                format!(
                                    "Timed out waiting for username field (preferred selectors for {}ms: {}; fallback selectors for {}ms: {})",
                                    preferred_window_ms,
                                    PREFERRED_USER_SELECTORS.join(", "),
                                    fallback_window_ms,
                                    FALLBACK_USER_SELECTORS.join(", ")
                                ),
                            )
                        })?
                }
            }
        }
    };
    check_origin(page, expected_origin.as_ref()).await?;
    fill(page, &user_selector, &username).await?;

    // Find and fill password
    let pass_sel = password_sel.unwrap_or_else(|| PASSWORD_SELECTOR.to_string());
    wait_for_any_selector(page, &[pass_sel.as_str()], auth_timeout_ms)
        .await?
        .ok_or_else(|| {
            error(
                "TIMEOUT",
                format!("Timed out waiting for password selector '{}'", pass_sel),
            )
        })?;
    check_origin(page, expected_origin.as_ref()).await?;
    // Only into a password field, which the page never shows back as text.
    let kind = page
        .call(
            "page.query",
            json!({"what": "attr", "name": "type", "target": {"selector": pass_sel}}),
        )
        .await?;
    if kind.get("value").and_then(Value::as_str) != Some("password") {
        return Err(error(
            "INVALID_TARGET",
            format!("'{pass_sel}' is not a password field; the password is typed only into one"),
        ));
    }
    fill(page, &pass_sel, &password)
        .await
        .map_err(|failure| hide(failure, &password))?;

    // Find and click submit
    let submit_selector = match submit_sel {
        Some(s) => wait_for_any_selector(page, &[s.as_str()], auth_timeout_ms)
            .await?
            .ok_or_else(|| {
                error(
                    "TIMEOUT",
                    format!("Timed out waiting for submit selector '{}'", s),
                )
            })?,
        None => wait_for_any_selector(page, &AUTO_SUBMIT_SELECTORS, auth_timeout_ms)
            .await?
            .ok_or_else(|| {
                error(
                    "TIMEOUT",
                    format!(
                        "Timed out waiting for submit button (tried selectors: {})",
                        AUTO_SUBMIT_SELECTORS.join(", ")
                    ),
                )
            })?,
    };
    check_origin(page, expected_origin.as_ref()).await?;
    let before = url_of(page).await.ok();
    page.call(
        "page.clickElement",
        json!({ "target": { "selector": submit_selector } }),
    )
    .await?;

    settle_after_submit(page, before, auth_timeout_ms).await;

    Ok(json!({ "loggedIn": true, "name": request.name }))
}

/// Polls `selectors` in order until one's first match is visible and enabled, or `timeout_ms`
/// passes (`None`). The probes are `page.query` reads, which show nothing on the page; an error
/// that waiting cannot fix (a blocked site, a closed tab, a selector that does not parse) ends the
/// wait.
async fn wait_for_any_selector(
    page: &impl Page,
    selectors: &[&str],
    timeout_ms: u64,
) -> Result<Option<String>, ActionError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        for selector in selectors {
            if usable(page, selector).await? {
                return Ok(Some((*selector).to_owned()));
            }
        }

        if Instant::now() >= deadline {
            return Ok(None);
        }

        tokio::time::sleep(SELECTOR_POLL).await;
    }
}

async fn usable(page: &impl Page, selector: &str) -> Result<bool, ActionError> {
    for what in ["visible", "enabled"] {
        let probed = page
            .call(
                "page.query",
                json!({ "target": { "selector": selector }, "what": what }),
            )
            .await;
        match probed {
            Ok(answer) if answer.get("value") == Some(&Value::Bool(true)) => {}
            Ok(_) => return Ok(false),
            Err(failure) if not_yet(&failure) => return Ok(false),
            Err(failure) => return Err(failure),
        }
    }
    Ok(true)
}

/// A probe failure the page may still get past: no match yet, or a document between loads.
fn not_yet(failure: &ActionError) -> bool {
    match failure.code.as_str() {
        "TARGET_NOT_FOUND" | "TAB_UNREACHABLE" | "TIMEOUT" => true,
        "ACTION_FAILED" => !failure.message.contains("is not a valid selector"),
        _ => false,
    }
}

async fn fill(page: &impl Page, selector: &str, value: &str) -> Result<(), ActionError> {
    page.call(
        "page.fillInput",
        json!({ "target": { "selector": selector }, "value": value }),
    )
    .await
    .map(drop)
}

/// The page's error with the secret it may have echoed replaced.
fn hide(mut failure: ActionError, secret: &str) -> ActionError {
    if !secret.is_empty() {
        failure.message = failure.message.replace(secret, HIDDEN);
    }
    failure
}

/// After the submit: until a navigation shows (the tab's URL changes, or its document is between
/// loads) and loads, for at most 10 s; with none, a 2 s settle, as agent-browser waits for a
/// `Page.frameNavigated` it never sees.
async fn settle_after_submit(page: &impl Page, before: Option<String>, auth_timeout_ms: u64) {
    let window_ms = auth_timeout_ms.min(POST_SUBMIT_WINDOW_MS);
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    let mut navigated = false;

    while Instant::now() < deadline {
        tokio::time::sleep(SELECTOR_POLL).await;
        match url_of(page).await {
            Ok(url) if before.as_deref() == Some(url.as_str()) => {}
            _ => {
                navigated = true;
                break;
            }
        }
    }

    if navigated {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        let timeout_ms = u64::try_from(left).unwrap_or(u64::MAX).clamp(100, 60_000);
        let _ = page
            .call(
                "page.waitFor",
                json!({ "load": "load", "timeoutMs": timeout_ms }),
            )
            .await;
    } else {
        let fallback_sleep_ms = auth_timeout_ms.min(POST_SUBMIT_FALLBACK_MS);
        if fallback_sleep_ms > 0 {
            tokio::time::sleep(Duration::from_millis(fallback_sleep_ms)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;
    use std::cell::RefCell;

    const TEST_KEY: [u8; 32] = [0xaa; 32];

    /// A vault in a fresh scratch folder, with no key yet.
    fn vault(name: &str) -> Vault {
        let dir = std::env::temp_dir().join(format!("vexdom-auth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Vault {
            dir: dir.join("vault"),
            key_file: dir.join(KEY_FILE_NAME),
            env_key: None,
        }
    }

    fn keyed(name: &str) -> Vault {
        Vault {
            env_key: Some("a".repeat(64)),
            ..vault(name)
        }
    }

    fn profile(name: &str, password: &str) -> AuthProfile {
        AuthProfile {
            name: name.to_string(),
            url: "https://example.com/login".to_string(),
            username: "user".to_string(),
            password: password.to_string(),
            username_selector: None,
            password_selector: None,
            submit_selector: None,
            created_at: None,
            last_login_at: None,
        }
    }

    #[test]
    fn test_validate_profile_name() {
        assert!(validate_profile_name("github").is_ok());
        assert!(validate_profile_name("my-app").is_ok());
        assert!(validate_profile_name("test_123").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("has space").is_err());
        assert!(validate_profile_name("../evil").is_err());
        assert!(validate_profile_name("foo/bar").is_err());
        assert_eq!(
            validate_profile_name("foo/bar").unwrap_err(),
            error(
                "INVALID_INPUT",
                "Invalid profile name 'foo/bar'. Must match /^[a-zA-Z0-9_-]+$/"
            )
        );
    }

    #[test]
    fn test_auth_profile_serialization() {
        let profile = AuthProfile {
            submit_selector: Some("button[type=submit]".to_string()),
            ..profile("test", "pass")
        };
        let json = serde_json::to_string(&profile).unwrap();
        let parsed: AuthProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(
            parsed.submit_selector,
            Some("button[type=submit]".to_string())
        );
        assert!(parsed.username_selector.is_none());
        assert!(json.contains("\"submitSelector\""));
        assert!(!json.contains("usernameSelector"));
        assert!(!format!("{parsed:?}").contains("pass\""));
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let encrypted_json = encrypt_profile(&TEST_KEY, &profile("roundtrip", "s3cret!")).unwrap();
        assert!(!encrypted_json.contains("s3cret!"));
        let decrypted = decrypt_profile(&TEST_KEY, encrypted_json.as_bytes()).unwrap();
        assert_eq!(decrypted.name, "roundtrip");
        assert_eq!(decrypted.password, "s3cret!");
    }

    #[test]
    fn a_profile_does_not_open_under_another_key() {
        let encrypted_json = encrypt_profile(&TEST_KEY, &profile("wrong", "s3cret!")).unwrap();
        let failure = decrypt_profile(&[0xbb; 32], encrypted_json.as_bytes()).unwrap_err();
        assert_eq!(failure.code, "ACTION_FAILED");
        assert!(failure.message.starts_with("Decryption failed: "));
        assert!(!failure.message.contains("s3cret!"));
        assert_eq!(
            decrypt_profile(
                &TEST_KEY,
                br#"{"name":"x","url":"","username":"","password":""}"#
            )
            .unwrap_err()
            .message,
            "Profile is not a valid encrypted payload"
        );
    }

    #[test]
    fn test_get_encryption_key_from_env() {
        let key = keyed("env-key").key().unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.iter().all(|&b| b == 0xaa));
    }

    #[test]
    fn test_ensure_encryption_key_rejects_malformed_env_key() {
        let vault = Vault {
            env_key: Some("nothex".to_string()),
            ..vault("malformed")
        };
        let failure = vault.ensure_key().unwrap_err();
        assert_eq!(failure.code, "VAULT_KEY_MISSING");
        assert!(
            failure
                .message
                .starts_with("VEXDOM_VAULT_KEY should be a 64-character")
        );
        assert!(!vault.key_file.exists());
        assert_matches!(
            save(&vault, &json!({"name": "x", "url": "u", "username": "u", "password": "p"})),
            Err(ActionError { code, .. }) if code == "VAULT_KEY_MISSING"
        );
        assert!(!vault.dir.exists());
    }

    #[test]
    fn the_first_save_creates_the_key_file_once() {
        let vault = vault("key-file");
        let input = json!({"name": "site", "url": "https://example.com/login", "username": "ada", "password": "s3cret!"});
        let first = save(&vault, &input).unwrap();
        assert_eq!(first["saved"], "site");
        assert_eq!(first["keyCreated"], true);
        assert!(
            first["notice"]
                .as_str()
                .unwrap()
                .starts_with("Auto-generated encryption key at ")
        );
        let hex = std::fs::read_to_string(&vault.key_file).unwrap();
        assert_eq!(hex.len(), 65, "64 hex digits and a newline");
        assert!(parse_key_hex(&hex).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &std::path::Path| {
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777
            };
            assert_eq!(mode(&vault.key_file), 0o600);
            assert_eq!(mode(&vault.profile_path("site")), 0o600);
            assert_eq!(mode(&vault.dir), 0o700);
        }
        let second = save(&vault, &input).unwrap();
        assert_eq!(second, json!({"saved": "site"}));
        assert_eq!(std::fs::read_to_string(&vault.key_file).unwrap(), hex);
        let stored = std::fs::read_to_string(vault.profile_path("site")).unwrap();
        assert!(!stored.contains("s3cret!"));
        assert_eq!(vault.load("site").unwrap().password, "s3cret!");
    }

    #[test]
    fn test_parse_key_hex_valid() {
        let hex = "ab".repeat(32);
        let key = parse_key_hex(&hex).unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.iter().all(|&b| b == 0xab));
        assert!(parse_key_hex(&format!("{hex}\n")).is_some());
    }

    #[test]
    fn test_parse_key_hex_invalid() {
        assert!(parse_key_hex("too_short").is_none());
        assert!(parse_key_hex(&"g".repeat(64)).is_none());
        assert!(parse_key_hex("").is_none());
    }

    #[test]
    fn test_decrypt_json_payload_format() {
        let profile = AuthProfile {
            url: "https://example.com/login".to_string(),
            username: "admin".to_string(),
            username_selector: Some("#email".to_string()),
            ..profile("json-test", "hunter2")
        };

        // Encrypt with aes-gcm, then build the JSON payload by hand, as Node.js would.
        let cipher = Aes256Gcm::new_from_slice(&TEST_KEY).unwrap();
        let iv = rand::bytes(IV_LEN);
        let nonce = Nonce::<Aes256Gcm>::try_from(iv.as_slice()).unwrap();
        let plaintext = serde_json::to_string(&profile).unwrap();
        let encrypted = cipher.encrypt(&nonce, plaintext.as_bytes()).unwrap();

        let tag_offset = encrypted.len() - TAG_LEN;
        let ciphertext = &encrypted[..tag_offset];
        let auth_tag = &encrypted[tag_offset..];

        let payload = format!(
            r#"{{"version":1,"encrypted":true,"iv":"{}","authTag":"{}","data":"{}"}}"#,
            STANDARD.encode(&iv),
            STANDARD.encode(auth_tag),
            STANDARD.encode(ciphertext),
        );

        let decrypted = decrypt_profile(&TEST_KEY, payload.as_bytes()).unwrap();
        assert_eq!(decrypted.name, "json-test");
        assert_eq!(decrypted.password, "hunter2");
        assert_eq!(decrypted.username_selector, Some("#email".to_string()));
    }

    #[test]
    fn test_encrypted_output_is_json_format() {
        let encrypted = encrypt_profile(&TEST_KEY, &profile("format-check", "pass")).unwrap();
        let parsed: Value = serde_json::from_str(&encrypted).unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["encrypted"], true);
        assert!(parsed["iv"].is_string());
        assert!(parsed["authTag"].is_string());
        assert!(parsed["data"].is_string());
    }

    #[tokio::test]
    async fn saved_logins_are_listed_and_shown_without_their_password() {
        let vault = keyed("crud");
        let opts = InvokeOpts::default();
        let call = async |action: &str, input: Value| {
            run(&vault, None, action, &input, &opts, &|_| {}).await
        };

        assert_eq!(
            call("bridge.authList", json!({})).await.unwrap(),
            json!({"profiles": []})
        );
        let saved = call(
            "bridge.authSave",
            json!({"name": "github", "url": "https://github.com/login", "username": "ada",
                   "password": "hunter2-secret", "submitSelector": "#go", "usernameSelector": ""}),
        )
        .await
        .unwrap();
        assert_eq!(saved, json!({"saved": "github"}));
        assert!(!vault.key_file.exists(), "the variable's key needs no file");
        call(
            "bridge.authSave",
            json!({"name": "another", "url": "https://example.com", "username": "bob", "password": "pw-two"}),
        )
        .await
        .unwrap();
        std::fs::write(vault.profile_path("broken"), "{}").unwrap();

        let listed = call("bridge.authList", json!({})).await.unwrap();
        assert_eq!(
            listed,
            json!({"profiles": [
                {"name": "another", "username": "bob", "url": "https://example.com"},
                {"name": "broken", "error": "Failed to decrypt"},
                {"name": "github", "username": "ada", "url": "https://github.com/login"},
            ]})
        );
        let shown = call("bridge.authShow", json!({"name": "github"}))
            .await
            .unwrap();
        assert_eq!(
            shown,
            json!({"profile": {"name": "github", "url": "https://github.com/login", "username": "ada",
                               "usernameSelector": null, "passwordSelector": null, "submitSelector": "#go"}})
        );
        for answer in [&listed, &shown] {
            let text = answer.to_string();
            assert!(!text.contains("hunter2-secret") && !text.contains("pw-two"));
        }

        assert_eq!(
            call("bridge.authDelete", json!({"name": "github"}))
                .await
                .unwrap(),
            json!({"deleted": "github"})
        );
        for action in ["bridge.authShow", "bridge.authDelete"] {
            assert_eq!(
                call(action, json!({"name": "github"})).await.unwrap_err(),
                error("AUTH_PROFILE_NOT_FOUND", "Auth profile 'github' not found")
            );
        }
        assert_matches!(
            call("bridge.authShow", json!({"name": "../evil"})).await,
            Err(ActionError { code, .. }) if code == "INVALID_INPUT"
        );
        assert_eq!(
            call(
                "bridge.authSave",
                json!({"name": "x", "url": "u", "username": "u"})
            )
            .await
            .unwrap_err(),
            error("INVALID_INPUT", "Missing 'password'")
        );
        assert_matches!(
            call("bridge.authNope", json!({})).await,
            Err(ActionError { code, .. }) if code == "UNKNOWN_ACTION"
        );
    }

    #[tokio::test]
    async fn a_vault_without_its_key_cannot_open_logins() {
        let saving = keyed("no-key");
        save(
            &saving,
            &json!({"name": "site", "url": "https://example.com", "username": "u", "password": "p"}),
        )
        .unwrap();
        let without = Vault {
            env_key: None,
            ..keyed("no-key-reader")
        };
        let without = Vault {
            dir: saving.dir.clone(),
            ..without
        };
        let failure = without.load("site").unwrap_err();
        assert_eq!(failure.code, "VAULT_KEY_MISSING");
        assert!(
            failure
                .message
                .starts_with("Encryption key required. Set VEXDOM_VAULT_KEY")
        );
        assert_eq!(
            without.list(),
            json!({"profiles": [{"name": "site", "error": "Failed to decrypt"}]})
        );
    }

    #[tokio::test]
    async fn login_needs_a_name_and_a_browser() {
        let vault = keyed("offline");
        let opts = InvokeOpts::default();
        assert_eq!(
            run(&vault, None, "bridge.authLogin", &json!({}), &opts, &|_| {})
                .await
                .unwrap_err(),
            error("INVALID_INPUT", "Missing 'name'")
        );
        assert_eq!(
            run(
                &vault,
                None,
                "bridge.authLogin",
                &json!({"name": "site"}),
                &opts,
                &|_| {}
            )
            .await
            .unwrap_err(),
            error("EXTENSION_OFFLINE", OFFLINE)
        );
        let request =
            LoginRequest::parse(&json!({"credentialProvider": "vault", "item": "Example"}))
                .unwrap();
        assert_eq!(request.name, "Example");
        assert_eq!(request.timeout_ms, DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn the_save_input_shows_no_password() {
        let input = json!({"name": "site", "password": "hunter2"});
        assert_eq!(
            shown("bridge.authSave", &input),
            json!({"name": "site", "password": "********"})
        );
        assert_eq!(shown("bridge.authLogin", &input), input);
    }

    #[test]
    fn test_auth_login_origin_ignores_path_query_and_fragment() {
        let first = auth_login_origin(
            "https://example.com/login?credential-path-sentinel#credential-fragment-sentinel",
            "credential",
        )
        .unwrap();
        let second = auth_login_origin(
            "https://example.com/consent?current-path-sentinel#current-fragment-sentinel",
            "current page",
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.0, "https://example.com");
    }

    #[test]
    fn test_auth_login_origin_normalizes_default_ports() {
        assert_eq!(
            auth_login_origin("http://example.com:80/login", "credential").unwrap(),
            auth_login_origin("http://example.com/flow", "current page").unwrap()
        );
        assert_eq!(
            auth_login_origin("https://example.com:443/login", "credential").unwrap(),
            auth_login_origin("https://example.com/flow", "current page").unwrap()
        );
    }

    #[test]
    fn test_auth_login_origin_rejects_mismatched_origins() {
        let expected = auth_login_origin("https://example.com/login", "credential").unwrap();
        assert_ne!(
            expected,
            auth_login_origin("http://example.com/login", "current page").unwrap()
        );
        assert_ne!(
            expected,
            auth_login_origin("https://id.example.com/login", "current page").unwrap()
        );
        assert_ne!(
            expected,
            auth_login_origin("https://example.com:8443/login", "current page").unwrap()
        );
    }

    #[test]
    fn test_auth_login_origin_rejects_malformed_opaque_and_non_http_urls() {
        for value in [
            "not a url",
            "about:blank",
            "data:text/html,login",
            "file:///tmp/login.html",
            "https://",
            "https:///login",
            "https:example.com/login",
        ] {
            let error = auth_login_origin(value, "credential").unwrap_err();
            assert!(error.contains("absolute HTTP(S) URL with a host"));
            assert!(!error.contains(value));
        }
    }

    #[test]
    fn test_auth_login_origin_mismatch_error_is_sanitized() {
        let current = auth_login_origin(
            "https://login.example.com/current-path-sentinel?current-query-sentinel#current-fragment-sentinel",
            "current page",
        )
        .unwrap();
        let expected = auth_login_origin(
            "https://example.com/credential-path-sentinel?credential-query-sentinel#credential-fragment-sentinel",
            "credential",
        )
        .unwrap();
        let error = ensure_auth_login_origin_matches(&current, &expected).unwrap_err();

        assert_eq!(
            error,
            "Current page origin https://login.example.com does not match credential origin https://example.com"
        );
        for sentinel in [
            "current-path-sentinel",
            "current-query-sentinel",
            "current-fragment-sentinel",
            "credential-path-sentinel",
            "credential-query-sentinel",
            "credential-fragment-sentinel",
        ] {
            assert!(!error.contains(sentinel));
        }
    }

    /// A tab that answers the login's calls: `present` selectors have a visible, enabled first
    /// match; a click moves the tab on; filling `refuse_fill` fails, echoing the value as a page
    /// error could.
    struct FakePage {
        url: RefCell<String>,
        present: Vec<&'static str>,
        refuse_fill: Option<&'static str>,
        calls: RefCell<Vec<(String, Value)>>,
    }

    impl FakePage {
        fn at(url: &str, present: &[&'static str]) -> Self {
            Self {
                url: RefCell::new(url.to_owned()),
                present: present.to_vec(),
                refuse_fill: None,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls_to(&self, action: &str) -> Vec<Value> {
            self.calls
                .borrow()
                .iter()
                .filter(|(called, _)| called == action)
                .map(|(_, input)| input.clone())
                .collect()
        }
    }

    impl Page for FakePage {
        async fn call(&self, action: &str, input: Value) -> ActionResult {
            self.calls
                .borrow_mut()
                .push((action.to_owned(), input.clone()));
            let selector = input.pointer("/target/selector").and_then(Value::as_str);
            match action {
                "page.navigate" => {
                    *self.url.borrow_mut() = input["url"].as_str().unwrap_or_default().to_owned();
                    Ok(json!({ "loaded": true }))
                }
                "page.query" if input["what"] == "url" => {
                    Ok(json!({ "value": self.url.borrow().clone() }))
                }
                "page.query" if input["what"] == "attr" => match selector {
                    Some(selector) if selector.contains("password") => {
                        Ok(json!({ "value": "password" }))
                    }
                    Some(selector) if self.present.contains(&selector) => {
                        Ok(json!({ "value": "text" }))
                    }
                    _ => failure("TARGET_NOT_FOUND", "No element matches"),
                },
                "page.query" => match selector {
                    Some(selector) if self.present.contains(&selector) => {
                        Ok(json!({ "value": true }))
                    }
                    _ => failure("TARGET_NOT_FOUND", "No element matches"),
                },
                "page.fillInput" if selector.is_some() && selector == self.refuse_fill => failure(
                    "INVALID_INPUT",
                    format!(
                        "the browser refused \"{}\" for this number input",
                        input["value"].as_str().unwrap_or_default()
                    ),
                ),
                "page.fillInput" => Ok(json!({ "length": 1 })),
                "page.clickElement" => {
                    let next = format!("{}/home", self.url.borrow());
                    *self.url.borrow_mut() = next;
                    Ok(json!({ "submits": true }))
                }
                "page.waitFor" => Ok(json!({})),
                other => failure("UNKNOWN_ACTION", other.to_owned()),
            }
        }
    }

    fn saved_login(name: &str) -> Vault {
        let vault = keyed(name);
        save(
            &vault,
            &json!({"name": "site", "url": "https://example.com/login", "username": "ada",
                    "password": "hunter2-secret"}),
        )
        .unwrap();
        vault
    }

    #[tokio::test(start_paused = true)]
    async fn a_login_is_never_typed_into_another_origin() {
        let vault = saved_login("login-elsewhere");
        let page = FakePage::at("https://example.com/", &["input[type=password]"]);
        let elsewhere =
            LoginRequest::parse(&json!({"name": "site", "url": "https://attacker.example/login"}))
                .unwrap();
        let failure = login(&page, &elsewhere, &vault, &|_| {}).await.unwrap_err();
        assert_eq!(failure.code, "INVALID_TARGET");
        assert!(page.calls.borrow().is_empty());
        let same =
            LoginRequest::parse(&json!({"name": "site", "url": "https://example.com/other-login"}))
                .unwrap();
        assert!(credential(&vault, &same).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn login_navigates_fills_and_submits() {
        let vault = saved_login("login");
        let page = FakePage::at(
            "https://elsewhere.example/",
            &[
                "input[name=username]",
                "input[type=password]",
                "button[type=submit]",
            ],
        );
        let request = LoginRequest::parse(&json!({"name": "site", "timeoutMs": 3000})).unwrap();

        let result = login(&page, &request, &vault, &|_| {}).await.unwrap();

        assert_eq!(result, json!({"loggedIn": true, "name": "site"}));
        assert!(!result.to_string().contains("hunter2-secret"));
        let calls = page.calls.borrow();
        assert_eq!(
            calls[0],
            (
                "page.navigate".to_owned(),
                json!({"url": "https://example.com/login"})
            )
        );
        drop(calls);
        assert_eq!(
            page.calls_to("page.fillInput"),
            [
                json!({"target": {"selector": "input[name=username]"}, "value": "ada"}),
                json!({"target": {"selector": "input[type=password]"}, "value": "hunter2-secret"}),
            ]
        );
        assert_eq!(
            page.calls_to("page.clickElement"),
            [json!({"target": {"selector": "button[type=submit]"}})]
        );
        assert_eq!(page.calls_to("page.waitFor").len(), 1, "the new page loads");
        // The preferred selectors were tried in order before the one present.
        let probed: Vec<String> = page
            .calls_to("page.query")
            .iter()
            .filter_map(|input| input.pointer("/target/selector").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        assert_eq!(probed[..6], PREFERRED_USER_SELECTORS[..6]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_given_selectors_win_over_the_saved_ones() {
        let vault = saved_login("selectors");
        let page = FakePage::at("https://example.com/", &["#u", "#password", "#go", "#q"]);
        let request = LoginRequest::parse(&json!({
            "name": "site", "usernameSelector": "#u", "passwordSelector": "#password", "submitSelector": "#go"
        }))
        .unwrap();
        login(&page, &request, &vault, &|_| {}).await.unwrap();
        let filled: Vec<Value> = page
            .calls_to("page.fillInput")
            .iter()
            .map(|input| input["target"]["selector"].clone())
            .collect();
        assert_eq!(filled, [json!("#u"), json!("#password")]);

        // A text field the page would show back never receives the password.
        let page = FakePage::at("https://example.com/", &["#u", "#q", "#go"]);
        let request = LoginRequest::parse(&json!({
            "name": "site", "usernameSelector": "#u", "passwordSelector": "#q"
        }))
        .unwrap();
        let failure = login(&page, &request, &vault, &|_| {}).await.unwrap_err();
        assert_eq!(failure.code, "INVALID_TARGET");
        assert_eq!(page.calls_to("page.fillInput").len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn no_navigate_refuses_a_page_on_another_origin_before_filling() {
        let vault = saved_login("origin");
        let page = FakePage::at(
            "https://evil.example/login",
            &[
                "input[type=email]",
                "input[type=password]",
                "button[type=submit]",
            ],
        );
        let request = LoginRequest::parse(&json!({"name": "site", "noNavigate": true})).unwrap();

        let failure = login(&page, &request, &vault, &|_| {}).await.unwrap_err();

        assert_eq!(
            failure,
            error(
                "INVALID_TARGET",
                "Current page origin https://evil.example does not match credential origin https://example.com"
            )
        );
        assert!(page.calls_to("page.navigate").is_empty());
        assert!(page.calls_to("page.fillInput").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn no_navigate_needs_a_web_page_before_reading_any_credential() {
        // No key: reading the login would fail differently.
        let vault = vault("blank");
        let page = FakePage::at("about:blank", &[]);
        let request = LoginRequest::parse(&json!({"name": "site", "noNavigate": true})).unwrap();
        assert_eq!(
            login(&page, &request, &vault, &|_| {}).await.unwrap_err(),
            error("INVALID_TARGET", NO_NAVIGATE_PAGE_ERROR)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn no_navigate_logs_in_on_the_credential_origin() {
        let vault = saved_login("same-origin");
        let page = FakePage::at(
            "https://example.com/signin",
            &[
                "input[type=email]",
                "input[type=password]",
                "button[type=submit]",
            ],
        );
        let request = LoginRequest::parse(&json!({"name": "site", "noNavigate": true})).unwrap();
        login(&page, &request, &vault, &|_| {}).await.unwrap();
        assert!(page.calls_to("page.navigate").is_empty());
        assert_eq!(page.calls_to("page.fillInput").len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_password_fill_never_echoes_the_password() {
        let vault = saved_login("refused");
        let mut page = FakePage::at(
            "https://example.com/",
            &[
                "input[type=email]",
                "input[type=password]",
                "button[type=submit]",
            ],
        );
        page.refuse_fill = Some("input[type=password]");
        let request = LoginRequest::parse(&json!({"name": "site"})).unwrap();

        let failure = login(&page, &request, &vault, &|_| {}).await.unwrap_err();

        assert_eq!(failure.code, "INVALID_INPUT");
        assert_eq!(
            failure.message,
            "the browser refused \"********\" for this number input"
        );
        assert!(page.calls_to("page.clickElement").is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_missing_username_field_times_out_with_both_selector_lists() {
        let vault = saved_login("timeout");
        let page = FakePage::at("https://example.com/", &[]);
        let request = LoginRequest::parse(&json!({"name": "site", "timeoutMs": 6000})).unwrap();

        let failure = login(&page, &request, &vault, &|_| {}).await.unwrap_err();

        assert_eq!(failure.code, "TIMEOUT");
        assert_eq!(
            failure.message,
            format!(
                "Timed out waiting for username field (preferred selectors for 5000ms: {}; fallback selectors for 1000ms: input[type=text], input:not([type]))",
                PREFERRED_USER_SELECTORS.join(", ")
            )
        );
        assert!(page.calls_to("page.fillInput").is_empty());
    }
}
