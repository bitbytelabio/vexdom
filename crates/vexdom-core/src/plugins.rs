//! External plugins, ported from agent-browser's `cli/src/plugins.rs` (f0a9835, Apache-2.0, see
//! NOTICE). A plugin runs out of process and answers one JSON request on its stdin with one JSON
//! response on its stdout (protocol `vexdom.plugin.v1`). Its stderr is never read, so nothing it
//! prints there can leak. Vexdom keeps browser automation, policy and redaction: a
//! `credential.read` plugin only resolves a login on demand, and `command.run` lets
//! `vexdom plugin run` reach a plugin's own request types. Plugins are configured only in
//! `~/.vexdom/config.json` (`plugins`) or by `VEXDOM_PLUGINS`; a project's `vexdom.json` never adds
//! one.
//!
//! This crate has no async runtime: a plugin runs on the calling thread against a deadline, and the
//! Bridge calls it from `spawn_blocking`.

use std::fmt;
use std::io::{Read as _, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config;

pub const PROTOCOL_VERSION: &str = "vexdom.plugin.v1";
pub const TYPE_PLUGIN_MANIFEST: &str = "plugin.manifest";
pub const CAPABILITY_CREDENTIAL_READ: &str = "credential.read";
pub const CAPABILITY_COMMAND_RUN: &str = "command.run";

/// The only variables a plugin inherits.
const PLUGIN_ENV: [&str; 8] = [
    "PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "TMPDIR", "SHELL",
];

/// A JSON array of plugin entries that replaces config.json's list.
pub const PLUGINS_ENV: &str = "VEXDOM_PLUGINS";

/// How long a credential lookup may run.
pub const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `vexdom plugin run` and manifest discovery may run.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// How often a plugin whose stdout has closed is checked for its exit.
const EXIT_POLL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CredentialResolveRequest<'a> {
    pub profile_name: &'a str,
    pub item_ref: Option<&'a str>,
    pub url: Option<&'a str>,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedCredential {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub otp: Option<String>,
    #[serde(default)]
    pub username_selector: Option<String>,
    #[serde(default)]
    pub password_selector: Option<String>,
    #[serde(default)]
    pub submit_selector: Option<String>,
}

/// The secrets stay out of any debug print.
impl fmt::Debug for ResolvedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedCredential")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .field("url", &self.url)
            .field("otp", &self.otp.as_ref().map(|_| "[redacted]"))
            .field("username_selector", &self.username_selector)
            .field("password_selector", &self.password_selector)
            .field("submit_selector", &self.submit_selector)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialPluginResponse {
    protocol: String,
    success: bool,
    #[serde(default)]
    credential: Option<ResolvedCredential>,
}

/// What a plugin says about itself in answer to `plugin.manifest`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: Option<String>,
    pub capabilities: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifestResponse {
    protocol: String,
    success: bool,
    #[serde(default)]
    manifest: Option<PluginManifest>,
    #[serde(default)]
    data: Option<PluginManifest>,
}

/// The plugins in effect, and what could not be read: `VEXDOM_PLUGINS` when it holds a JSON array
/// of entries (it replaces config.json's list), else config.json's `plugins`.
pub fn configured() -> (Vec<PluginConfig>, Vec<String>) {
    resolve(std::env::var(PLUGINS_ENV).ok(), stored_entries())
}

fn resolve(
    env: Option<String>,
    stored: Result<Vec<Value>, String>,
) -> (Vec<PluginConfig>, Vec<String>) {
    let mut warnings = Vec::new();
    if let Some(raw) = env {
        match serde_json::from_str::<Vec<PluginConfig>>(&raw) {
            Ok(plugins) => return (plugins, warnings),
            Err(error) => warnings.push(format!("invalid {PLUGINS_ENV} value: {error}")),
        }
    }
    let plugins = match stored {
        Ok(entries) => match serde_json::from_value::<Vec<PluginConfig>>(Value::Array(entries)) {
            Ok(plugins) => plugins,
            Err(error) => {
                warnings.push(format!(
                    "ignoring the plugins in {}: {error}",
                    config::config_path().display()
                ));
                Vec::new()
            }
        },
        Err(error) => {
            warnings.push(error);
            Vec::new()
        }
    };
    (plugins, warnings)
}

/// config.json's `plugins` entries as stored, each kept whole so rewriting the list loses no field
/// of another entry. No file or no key is an empty list.
pub fn stored_entries() -> Result<Vec<Value>, String> {
    match config::read_strict()?.shift_remove("plugins") {
        None => Ok(Vec::new()),
        Some(Value::Array(entries)) => Ok(entries),
        Some(_) => Err(format!(
            "Config {} field 'plugins' must be an array",
            config::config_path().display()
        )),
    }
}

pub fn find_plugin<'a>(plugins: &'a [PluginConfig], name: &str) -> Option<&'a PluginConfig> {
    plugins.iter().rev().find(|p| p.name == name)
}

pub fn plugin_has_capability(plugin: &PluginConfig, capability: &str) -> bool {
    plugin.capabilities.iter().any(|c| c == capability)
}

fn validate_plugin(plugin: &PluginConfig, capability: &str) -> Result<(), String> {
    if plugin.name.trim().is_empty() {
        return Err("Plugin entry is missing a name".to_string());
    }
    if plugin.command.trim().is_empty() {
        return Err(format!("Plugin '{}' is missing a command", plugin.name));
    }
    if !plugin_has_capability(plugin, capability) {
        return Err(format!(
            "Plugin '{}' does not declare required capability '{}'",
            plugin.name, capability
        ));
    }
    Ok(())
}

/// One request of `request_type` under a capability the plugin declares. With
/// `expose_plugin_error` a failure carries the plugin's own `error` text and protocol, which only
/// `vexdom plugin` commands the user runs ask for.
pub fn invoke_plugin(
    plugin: &PluginConfig,
    request_type: &str,
    capability: &str,
    request: Value,
    timeout: Duration,
    expose_plugin_error: bool,
) -> Result<Value, String> {
    validate_plugin(plugin, capability)?;
    let payload = json!({
        "protocol": PROTOCOL_VERSION,
        "type": request_type,
        "capability": capability,
        "request": request,
    });
    invoke_plugin_process(plugin, &payload, timeout, expose_plugin_error)
}

fn invoke_plugin_process(
    plugin: &PluginConfig,
    payload: &Value,
    timeout: Duration,
    expose_plugin_error: bool,
) -> Result<Value, String> {
    let input = serde_json::to_vec(payload)
        .map_err(|e| format!("Failed to encode plugin request: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(&plugin.command);
    // A plugin sees no secrets from the Bridge's environment (the vault key, API keys): only what
    // finding and running a program needs, from the state folder.
    command.env_clear();
    for key in PLUGIN_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let state = crate::paths::state_dir();
    if state.is_dir() {
        command.current_dir(state);
    }
    let mut child = command
        .args(&plugin.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start plugin '{}': {}", plugin.name, e))?;

    // The request is written whole, then stdin closes so the plugin sees its end.
    let written = child
        .stdin
        .take()
        .map_or(Ok(()), |mut stdin| stdin.write_all(&input));
    if let Err(e) = written {
        stop(&mut child);
        return Err(format!("Failed to write plugin request: {}", e));
    }

    let (status, stdout) = wait_until(child, deadline).map_err(|outcome| match outcome {
        Outcome::TimedOut => format!("Plugin '{}' timed out", plugin.name),
        Outcome::Failed(e) => format!("Plugin '{}' failed: {}", plugin.name, e),
    })?;

    if !status.success() {
        return Err(format!("Plugin '{}' exited unsuccessfully", plugin.name));
    }

    let response: Value = serde_json::from_slice(&stdout)
        .map_err(|_| format!("Plugin '{}' returned invalid JSON", plugin.name))?;
    let protocol = response
        .get("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if protocol != PROTOCOL_VERSION {
        if !expose_plugin_error {
            return Err(format!(
                "Plugin '{}' used unsupported protocol",
                plugin.name
            ));
        }
        return Err(format!(
            "Plugin '{}' used unsupported protocol '{}'",
            plugin.name, protocol
        ));
    }
    if !response
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        if expose_plugin_error {
            let error = response
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("plugin returned success=false");
            return Err(format!("Plugin '{}' failed: {}", plugin.name, error));
        }
        return Err(format!("Plugin '{}' returned success=false", plugin.name));
    }

    Ok(response)
}

enum Outcome {
    TimedOut,
    Failed(std::io::Error),
}

/// The exit status and all of stdout, or the plugin killed once `deadline` passes. Stdout is read
/// on its own thread so a full pipe never stalls the plugin; a reader left behind by a killed
/// plugin's own children ends when they close the pipe.
fn wait_until(mut child: Child, deadline: Instant) -> Result<(ExitStatus, Vec<u8>), Outcome> {
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let read = match stdout {
            Some(mut pipe) => pipe.read_to_end(&mut out).map(|_| out),
            None => Ok(out),
        };
        let _ = sender.send(read);
    });
    let stdout = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(stdout)) => stdout,
        Ok(Err(error)) => {
            stop(&mut child);
            return Err(Outcome::Failed(error));
        }
        Err(_) => {
            stop(&mut child);
            return Err(Outcome::TimedOut);
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status, stdout)),
            Ok(None) if Instant::now() >= deadline => {
                stop(&mut child);
                return Err(Outcome::TimedOut);
            }
            Ok(None) => std::thread::sleep(EXIT_POLL),
            Err(error) => {
                stop(&mut child);
                return Err(Outcome::Failed(error));
            }
        }
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A login from the `credential.read` plugin named `provider`. Failures never carry what the
/// plugin said: it may have echoed the secret it was asked for.
pub fn resolve_credential_with_plugins(
    provider: &str,
    plugins: &[PluginConfig],
    request: CredentialResolveRequest<'_>,
) -> Result<ResolvedCredential, String> {
    let plugin = find_plugin(plugins, provider)
        .ok_or_else(|| format!("Credential plugin '{}' is not configured", provider))?;
    let response = invoke_plugin(
        plugin,
        "credential.resolve",
        CAPABILITY_CREDENTIAL_READ,
        json!({
            "profileName": request.profile_name,
            "itemRef": request.item_ref,
            "url": request.url,
        }),
        CREDENTIAL_TIMEOUT,
        false,
    )?;
    let response: CredentialPluginResponse = serde_json::from_value(response)
        .map_err(|_| format!("Credential plugin '{}' returned invalid JSON", provider))?;
    if response.protocol != PROTOCOL_VERSION {
        return Err(format!(
            "Credential plugin '{}' used unsupported protocol '{}'",
            provider, response.protocol
        ));
    }
    if !response.success {
        return Err(format!(
            "Credential plugin '{}' could not resolve credentials",
            provider
        ));
    }
    let credential = response
        .credential
        .ok_or_else(|| format!("Credential plugin '{}' returned no credential", provider))?;
    if credential.username.is_empty() || credential.password.is_empty() {
        return Err(format!(
            "Credential plugin '{}' returned an incomplete credential",
            provider
        ));
    }
    Ok(credential)
}

/// Asks a plugin for its name and capabilities (`plugin.manifest`), for `vexdom plugin add`.
pub fn discover_plugin_manifest(plugin: &PluginConfig) -> Result<PluginManifest, String> {
    let payload = json!({
        "protocol": PROTOCOL_VERSION,
        "type": TYPE_PLUGIN_MANIFEST,
        "capability": TYPE_PLUGIN_MANIFEST,
        "request": {},
    });

    let response = invoke_plugin_process(plugin, &payload, COMMAND_TIMEOUT, true)?;
    let response: PluginManifestResponse = serde_json::from_value(response)
        .map_err(|_| format!("Plugin '{}' returned invalid manifest JSON", plugin.name))?;
    if response.protocol != PROTOCOL_VERSION {
        return Err(format!(
            "Plugin '{}' used unsupported protocol '{}'",
            plugin.name, response.protocol
        ));
    }
    if !response.success {
        return Err(format!("Plugin '{}' returned success=false", plugin.name));
    }
    response
        .manifest
        .or(response.data)
        .ok_or_else(|| format!("Plugin '{}' returned no manifest", plugin.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;
    use crate::paths;

    /// A fresh folder for one test's plugin scripts.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vexdom-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An executable `sh` script in `dir`.
    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn plugin(name: &str, command: &str, capabilities: &[&str]) -> PluginConfig {
        PluginConfig {
            name: name.to_string(),
            command: command.to_string(),
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            ..PluginConfig::default()
        }
    }

    fn request() -> CredentialResolveRequest<'static> {
        CredentialResolveRequest {
            profile_name: "example",
            item_ref: Some("Example"),
            url: None,
        }
    }

    #[test]
    fn find_plugin_prefers_last_entry() {
        let plugins = vec![
            plugin("vault", "first", &[CAPABILITY_CREDENTIAL_READ]),
            plugin("vault", "second", &[CAPABILITY_CREDENTIAL_READ]),
        ];

        assert_eq!(find_plugin(&plugins, "vault").unwrap().command, "second");
        assert!(find_plugin(&plugins, "other").is_none());
    }

    #[test]
    fn a_plugin_needs_a_name_a_command_and_the_capability() {
        let check = |entry: PluginConfig| validate_plugin(&entry, CAPABILITY_CREDENTIAL_READ);
        assert_eq!(
            check(plugin(" ", "x", &[CAPABILITY_CREDENTIAL_READ])).unwrap_err(),
            "Plugin entry is missing a name"
        );
        assert_eq!(
            check(plugin("vault", "", &[CAPABILITY_CREDENTIAL_READ])).unwrap_err(),
            "Plugin 'vault' is missing a command"
        );
        assert_eq!(
            check(plugin("vault", "x", &[CAPABILITY_COMMAND_RUN])).unwrap_err(),
            "Plugin 'vault' does not declare required capability 'credential.read'"
        );
        assert!(check(plugin("vault", "x", &[CAPABILITY_CREDENTIAL_READ])).is_ok());
        // The capability check comes before anything runs.
        let err = invoke_plugin(
            &plugin("vault", "/nonexistent/vault", &[CAPABILITY_COMMAND_RUN]),
            "credential.resolve",
            CAPABILITY_CREDENTIAL_READ,
            json!({}),
            Duration::from_secs(1),
            false,
        )
        .unwrap_err();
        assert!(err.contains("does not declare required capability"));
    }

    #[test]
    fn plugin_entries_parse_with_defaults() {
        let entries: Vec<PluginConfig> = serde_json::from_value(json!([
            {"name": "onepassword", "command": "vexdom-plugin-1password",
             "args": ["--account", "team"], "capabilities": ["credential.read"]},
            {"name": "bare"}
        ]))
        .unwrap();
        assert_eq!(entries[0].args, ["--account", "team"]);
        assert_eq!(entries[0].capabilities, [CAPABILITY_CREDENTIAL_READ]);
        assert_eq!(entries[1].command, "");
        assert!(entries[1].capabilities.is_empty());
        assert_eq!(
            serde_json::to_value(&entries[1]).unwrap(),
            json!({"name": "bare", "command": "", "args": [], "capabilities": []})
        );
    }

    #[test]
    fn the_environment_list_replaces_the_config_list() {
        let stored = || Ok(vec![json!({"name": "stored", "command": "x"})]);
        let (plugins, warnings) =
            resolve(Some(r#"[{"name":"env","command":"y"}]"#.into()), stored());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "env");
        assert!(warnings.is_empty());

        let (plugins, warnings) = resolve(Some("not json".into()), stored());
        assert_eq!(plugins[0].name, "stored");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("invalid VEXDOM_PLUGINS value: "));

        let (plugins, warnings) = resolve(None, Err("Config x is invalid JSON: y".into()));
        assert!(plugins.is_empty());
        assert_eq!(warnings, ["Config x is invalid JSON: y"]);

        let (plugins, warnings) = resolve(None, Ok(vec![json!("not an entry")]));
        assert!(plugins.is_empty());
        assert!(warnings[0].starts_with("ignoring the plugins in "));
    }

    #[test]
    fn plugins_round_trip_through_config_json() {
        let _home = scratch_home("plugins");
        assert_eq!(stored_entries().unwrap(), Vec::<Value>::new());
        paths::write_private_json(
            &config::config_path(),
            &json!({"theme": "ember", "plugins": [{"name": "old", "command": "x", "extra": 1}], "keep": true}),
        )
        .unwrap();
        let mut entries = stored_entries().unwrap();
        assert_eq!(entries[0]["extra"], 1);
        entries.push(
            json!({"name": "vault", "command": "vault-cli", "capabilities": ["credential.read"]}),
        );
        config::write_plugins(&Value::Array(entries)).unwrap();

        let text = std::fs::read_to_string(config::config_path()).unwrap();
        let written: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["theme"], "ember");
        assert_eq!(written["keep"], true);
        assert_eq!(written["plugins"][0]["extra"], 1);
        assert_eq!(written["plugins"][1]["name"], "vault");
        // The key keeps its place among the others.
        assert!(text.find("\"theme\"").unwrap() < text.find("\"plugins\"").unwrap());
        assert!(text.find("\"plugins\"").unwrap() < text.find("\"keep\"").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config::config_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let (plugins, warnings) = resolve(None, stored_entries());
        assert!(warnings.is_empty());
        assert_eq!(plugins[1].capabilities, [CAPABILITY_CREDENTIAL_READ]);

        paths::write_private(&config::config_path(), b"{\"plugins\": {}}").unwrap();
        assert!(
            stored_entries()
                .unwrap_err()
                .ends_with("field 'plugins' must be an array")
        );
        paths::write_private(&config::config_path(), b"{not json").unwrap();
        assert!(stored_entries().unwrap_err().contains("is invalid JSON"));
        // A config.json that cannot be read is never replaced.
        assert!(
            config::write_plugins(&json!([]))
                .unwrap_err()
                .contains("is invalid JSON")
        );
        assert_eq!(std::fs::read(config::config_path()).unwrap(), b"{not json");
        paths::write_private(&config::config_path(), b"[]").unwrap();
        assert!(
            config::write_plugins(&json!([]))
                .unwrap_err()
                .ends_with("must be a JSON object")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_credential_uses_configured_stdio_plugin() {
        let dir = scratch("credential");
        let command = script(
            &dir,
            "mock-credential-plugin",
            r#"#!/bin/sh
cat >/dev/null
printf '%s' '{"protocol":"vexdom.plugin.v1","success":true,"credential":{"username":"user","password":"pass","url":"https://example.com/login"}}'
"#,
        );
        let registry = serde_json::to_string(&vec![plugin(
            "mock",
            &command,
            &[CAPABILITY_CREDENTIAL_READ],
        )])
        .unwrap();

        let (plugins, _) = resolve(Some(registry), Ok(Vec::new()));
        let credential = resolve_credential_with_plugins("mock", &plugins, request()).unwrap();

        assert_eq!(credential.username, "user");
        assert_eq!(credential.password, "pass");
        assert_eq!(credential.url.as_deref(), Some("https://example.com/login"));
        assert!(!format!("{credential:?}").contains("pass\""));
    }

    #[cfg(unix)]
    #[test]
    fn the_credential_request_names_the_profile_and_item() {
        let dir = scratch("request");
        let seen = dir.join("request.json");
        let command = script(
            &dir,
            "recording-plugin",
            &format!(
                r#"#!/bin/sh
cat > '{}'
printf '%s' '{{"protocol":"vexdom.plugin.v1","success":true,"credential":{{"username":"u","password":"p"}}}}'
"#,
                seen.display()
            ),
        );
        let plugins = vec![plugin("mock", &command, &[CAPABILITY_CREDENTIAL_READ])];
        resolve_credential_with_plugins("mock", &plugins, request()).unwrap();
        let sent: Value = serde_json::from_slice(&std::fs::read(seen).unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({
                "protocol": "vexdom.plugin.v1",
                "type": "credential.resolve",
                "capability": "credential.read",
                "request": {"profileName": "example", "itemRef": "Example", "url": null},
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn credential_plugin_failure_does_not_echo_plugin_error() {
        let dir = scratch("failing");
        let command = script(
            &dir,
            "mock-failing-credential-plugin",
            r#"#!/bin/sh
cat >/dev/null
echo 'secret-token-value' >&2
printf '%s' '{"protocol":"vexdom.plugin.v1","success":false,"error":"secret-token-value"}'
"#,
        );
        let plugins = vec![plugin("mock", &command, &[CAPABILITY_CREDENTIAL_READ])];

        let err = resolve_credential_with_plugins("mock", &plugins, request()).unwrap_err();

        assert!(err.contains("success=false"));
        assert!(!err.contains("secret-token-value"));
        assert_eq!(
            resolve_credential_with_plugins("absent", &plugins, request()).unwrap_err(),
            "Credential plugin 'absent' is not configured"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_incomplete_credential_is_refused() {
        let dir = scratch("incomplete");
        let command = script(
            &dir,
            "mock-incomplete-plugin",
            r#"#!/bin/sh
cat >/dev/null
printf '%s' '{"protocol":"vexdom.plugin.v1","success":true,"credential":{"username":"user","password":""}}'
"#,
        );
        let plugins = vec![plugin("mock", &command, &[CAPABILITY_CREDENTIAL_READ])];
        assert_eq!(
            resolve_credential_with_plugins("mock", &plugins, request()).unwrap_err(),
            "Credential plugin 'mock' returned an incomplete credential"
        );
    }

    #[cfg(unix)]
    #[test]
    fn plugin_protocol_error_obeys_redaction_setting() {
        let dir = scratch("protocol");
        let command = script(
            &dir,
            "fixture-plugin",
            r#"#!/bin/sh
cat >/dev/null
printf '%s' '{"protocol":"fixture-invalid-v0","success":false,"error":"fixture diagnostic"}'
"#,
        );
        let plugin = plugin("fixture", &command, &[]);
        let timeout = Duration::from_secs(5);
        let redacted = invoke_plugin_process(&plugin, &json!({}), timeout, false).unwrap_err();
        let detailed = invoke_plugin_process(&plugin, &json!({}), timeout, true).unwrap_err();

        assert_eq!(redacted, "Plugin 'fixture' used unsupported protocol");
        assert_eq!(
            detailed,
            "Plugin 'fixture' used unsupported protocol 'fixture-invalid-v0'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_command_is_reported_by_name_only() {
        let dir = scratch("exit");
        let command = script(
            &dir,
            "exit-plugin",
            "#!/bin/sh\ncat >/dev/null\necho 'secret' >&2\nexit 3\n",
        );
        let entry = plugin("exits", &command, &[CAPABILITY_COMMAND_RUN]);
        let timeout = Duration::from_secs(5);
        assert_eq!(
            invoke_plugin(
                &entry,
                "x.run",
                CAPABILITY_COMMAND_RUN,
                json!({}),
                timeout,
                true
            )
            .unwrap_err(),
            "Plugin 'exits' exited unsuccessfully"
        );
        let missing = plugin("missing", "/nonexistent/plugin", &[CAPABILITY_COMMAND_RUN]);
        assert!(
            invoke_plugin(
                &missing,
                "x.run",
                CAPABILITY_COMMAND_RUN,
                json!({}),
                timeout,
                true
            )
            .unwrap_err()
            .starts_with("Failed to start plugin 'missing': ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_manifest_names_the_plugin_and_its_capabilities() {
        let dir = scratch("manifest");
        let command = script(
            &dir,
            "manifest-plugin",
            r#"#!/bin/sh
cat >/dev/null
printf '%s' '{"protocol":"vexdom.plugin.v1","success":true,"manifest":{"name":"vault","capabilities":["credential.read"],"description":"Logins from a vault"}}'
"#,
        );
        let manifest = discover_plugin_manifest(&plugin("provisional", &command, &[])).unwrap();
        assert_eq!(
            manifest,
            PluginManifest {
                name: Some("vault".into()),
                capabilities: vec![CAPABILITY_CREDENTIAL_READ.into()],
                description: Some("Logins from a vault".into()),
            }
        );

        let command = script(
            &dir,
            "data-plugin",
            r#"#!/bin/sh
cat >/dev/null
printf '%s' '{"protocol":"vexdom.plugin.v1","success":true,"data":{"capabilities":["command.run"]}}'
"#,
        );
        let manifest = discover_plugin_manifest(&plugin("data", &command, &[])).unwrap();
        assert_eq!(manifest.name, None);
        assert_eq!(manifest.capabilities, [CAPABILITY_COMMAND_RUN]);

        let command = script(
            &dir,
            "bare-plugin",
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"protocol\":\"vexdom.plugin.v1\",\"success\":true}'\n",
        );
        assert_eq!(
            discover_plugin_manifest(&plugin("bare", &command, &[])).unwrap_err(),
            "Plugin 'bare' returned no manifest"
        );
    }

    #[cfg(unix)]
    #[test]
    fn timed_out_plugin_is_killed() {
        let dir = scratch("slow");
        let marker_path = dir.join("plugin-finished");
        let command = script(
            &dir,
            "mock-slow-plugin",
            r#"#!/bin/sh
cat >/dev/null
sleep 2
printf done > "$1"
printf '%s' '{"protocol":"vexdom.plugin.v1","success":true,"data":{}}'
"#,
        );
        let plugin = PluginConfig {
            args: vec![marker_path.to_string_lossy().to_string()],
            ..plugin("slow", &command, &[CAPABILITY_COMMAND_RUN])
        };

        let err = invoke_plugin(
            &plugin,
            "slow.run",
            CAPABILITY_COMMAND_RUN,
            json!({}),
            Duration::from_secs(1),
            true,
        )
        .unwrap_err();

        assert!(err.contains("timed out"));
        std::thread::sleep(Duration::from_millis(2_500));
        assert!(!marker_path.exists());
    }
}
