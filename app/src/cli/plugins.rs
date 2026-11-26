//! `vexdom plugin add|list|show|run`, ported from agent-browser's `cli/src/plugins.rs` (f0a9835,
//! Apache-2.0, see NOTICE): its texts and JSON shapes. Plugins live only in
//! `~/.vexdom/config.json` (or `VEXDOM_PLUGINS`), never in a project's `vexdom.json`; a reference
//! that is a path to a local executable registers that file itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use vexdom_core::config;
use vexdom_core::plugins::{
    self, CAPABILITY_COMMAND_RUN, CAPABILITY_CREDENTIAL_READ, PluginConfig, PluginManifest,
    TYPE_PLUGIN_MANIFEST,
};

use super::Args;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PluginSourceKind {
    Npm,
    Github,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginSource {
    kind: PluginSourceKind,
    reference: String,
    install_spec: String,
    source: String,
}

struct PluginAddOptions {
    reference: String,
    name: Option<String>,
    capabilities: Vec<String>,
    no_manifest: bool,
}

fn is_path_reference(reference: &str) -> bool {
    ["./", "../", "/", "~/"]
        .iter()
        .any(|prefix| reference.starts_with(prefix))
}

fn parse_plugin_source(reference: &str) -> Result<PluginSource, String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err("plugin add requires a package or repository reference".to_string());
    }
    if is_path_reference(trimmed) {
        let path = match trimmed.strip_prefix("~/") {
            Some(rest) => vexdom_core::paths::home_dir().join(rest),
            None => PathBuf::from(trimmed),
        };
        let absolute = std::fs::canonicalize(&path)
            .ok()
            .filter(|path| path.is_file())
            .ok_or_else(|| format!("plugin add: no file at {}", path.display()))?;
        let absolute = absolute.to_string_lossy().into_owned();
        return Ok(PluginSource {
            kind: PluginSourceKind::Path,
            reference: absolute.clone(),
            install_spec: absolute.clone(),
            source: format!("path:{}", absolute),
        });
    }
    if trimmed.starts_with('@') {
        return Ok(PluginSource {
            kind: PluginSourceKind::Npm,
            reference: trimmed.to_string(),
            install_spec: trimmed.to_string(),
            source: format!("npm:{}", trimmed),
        });
    }

    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && !trimmed.starts_with('.')
        && !trimmed.contains(':')
    {
        return Ok(PluginSource {
            kind: PluginSourceKind::Github,
            reference: trimmed.to_string(),
            install_spec: format!("github:{}", trimmed),
            source: format!("github:{}", trimmed),
        });
    }

    Ok(PluginSource {
        kind: PluginSourceKind::Npm,
        reference: trimmed.to_string(),
        install_spec: trimmed.to_string(),
        source: format!("npm:{}", trimmed),
    })
}

fn parse_plugin_add_args(args: &[String]) -> Result<PluginAddOptions, String> {
    let mut reference = None;
    let mut name = None;
    let mut capabilities = Vec::new();
    let mut no_manifest = false;
    let mut i = 2;

    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("plugin add --name requires a value".to_string());
                };
                name = Some(value.clone());
                i += 1;
            }
            "--capability" | "--capabilities" => {
                let Some(value) = args.get(i + 1) else {
                    return Err(format!("plugin add {} requires a value", args[i]));
                };
                capabilities.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                );
                i += 1;
            }
            // The one place plugins are kept.
            "--global" => {}
            "--project" => {
                return Err(
                    "plugins are kept only in ~/.vexdom/config.json; a project's vexdom.json cannot add them"
                        .to_string(),
                );
            }
            "--no-manifest" => {
                no_manifest = true;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag '{}' for plugin add", other));
            }
            value => {
                if reference.is_some() {
                    return Err(format!("unexpected argument '{}' for plugin add", value));
                }
                reference = Some(value.to_string());
            }
        }
        i += 1;
    }

    Ok(PluginAddOptions {
        reference: reference
            .ok_or_else(|| "plugin add requires a package or repository reference".to_string())?,
        name,
        capabilities,
        no_manifest,
    })
}

fn derive_plugin_name(source: &PluginSource) -> String {
    let raw = match source.kind {
        PluginSourceKind::Npm => package_name_without_version(&source.reference),
        PluginSourceKind::Github | PluginSourceKind::Path => source
            .reference
            .rsplit('/')
            .next()
            .unwrap_or(&source.reference)
            .to_string(),
    };
    raw.strip_prefix("vexdom-plugin-")
        .or_else(|| raw.strip_prefix("plugin-"))
        .or_else(|| raw.strip_suffix("-plugin"))
        .unwrap_or(&raw)
        .to_string()
}

fn package_name_without_version(reference: &str) -> String {
    if reference.starts_with('@') {
        let mut parts = reference.splitn(2, '/');
        let scope = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or(scope);
        return name
            .rsplit_once('@')
            .map(|(pkg, _)| pkg)
            .unwrap_or(name)
            .to_string();
    }
    reference.split('@').next().unwrap_or(reference).to_string()
}

fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("plugin name cannot be empty".to_string());
    }
    if name.chars().any(|c| c.is_whitespace() || c == ':') {
        return Err("plugin name cannot contain whitespace or ':'".to_string());
    }
    Ok(())
}

/// How the plugin is started: a local file itself, a package through `npx`.
fn command_for(source: &PluginSource) -> (String, Vec<String>) {
    match source.kind {
        PluginSourceKind::Path => (source.install_spec.clone(), Vec::new()),
        PluginSourceKind::Npm | PluginSourceKind::Github => (
            "npx".to_string(),
            vec!["-y".to_string(), source.install_spec.clone()],
        ),
    }
}

/// The stored entries with `plugin` in place of any entry of its name.
fn upsert_entries(mut entries: Vec<Value>, plugin: &PluginConfig) -> Result<Vec<Value>, String> {
    entries.retain(|entry| {
        entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(|name| name != plugin.name)
            .unwrap_or(true)
    });
    entries.push(
        serde_json::to_value(plugin)
            .map_err(|e| format!("Failed to encode plugin config: {}", e))?,
    );
    Ok(entries)
}

fn upsert_plugin_config(plugin: &PluginConfig) -> Result<PathBuf, String> {
    let entries = upsert_entries(plugins::stored_entries()?, plugin)?;
    config::write_plugins(&Value::Array(entries))?;
    Ok(config::config_path())
}

fn build_plugin_config_from_add(
    source: &PluginSource,
    options: &PluginAddOptions,
    manifest: Option<PluginManifest>,
) -> Result<PluginConfig, String> {
    let manifest_name = manifest.as_ref().and_then(|m| m.name.clone());
    let name = options
        .name
        .clone()
        .or(manifest_name)
        .unwrap_or_else(|| derive_plugin_name(source));
    validate_plugin_name(&name)?;

    let capabilities = if options.capabilities.is_empty() {
        manifest.map(|m| m.capabilities).unwrap_or_default()
    } else {
        options.capabilities.clone()
    };
    if capabilities.is_empty() {
        return Err(
            "Plugin did not declare capabilities; rerun with --capability <name> or use a plugin that supports plugin.manifest"
                .to_string(),
        );
    }

    let (command, args) = command_for(source);
    Ok(PluginConfig {
        name,
        command,
        args,
        capabilities,
        source: Some(source.source.clone()),
    })
}

fn print_plugin_added(plugin: &PluginConfig, path: &Path, json_output: bool) {
    if json_output {
        println!(
            "{}",
            json!({
                "success": true,
                "plugin": plugin,
                "configPath": path.display().to_string(),
            })
        );
        return;
    }

    println!("Added plugin '{}'", plugin.name);
    println!("Config: {}", path.display());
    if let Some(source) = &plugin.source {
        println!("Source: {}", source);
    }
    println!("Command: {} {}", plugin.command, plugin.args.join(" "));
    println!("Capabilities: {}", plugin.capabilities.join(", "));
}

fn add_plugin_command(args: &[String], json_output: bool) -> Result<(), String> {
    let options = parse_plugin_add_args(args)?;
    let source = parse_plugin_source(&options.reference)?;
    let (command, command_args) = command_for(&source);
    let provisional = PluginConfig {
        name: options
            .name
            .clone()
            .unwrap_or_else(|| derive_plugin_name(&source)),
        command,
        args: command_args,
        capabilities: Vec::new(),
        source: Some(source.source.clone()),
    };

    let manifest = if options.no_manifest {
        None
    } else {
        match plugins::discover_plugin_manifest(&provisional) {
            Ok(manifest) => Some(manifest),
            Err(e) if options.capabilities.is_empty() => {
                return Err(format!(
                    "PLUGIN_FAILED: {}. Rerun with --capability <name> to add the plugin without manifest discovery.",
                    e
                ));
            }
            Err(_) => None,
        }
    };

    let plugin = build_plugin_config_from_add(&source, &options, manifest)?;
    let path = upsert_plugin_config(&plugin)?;
    print_plugin_added(&plugin, &path, json_output);
    Ok(())
}

/// `vexdom plugin …`; `--json` is the global flag, taken out before the subcommand's own parsing.
pub(super) fn run(args: &Args) -> ExitCode {
    let json_output = args.json();
    let clean: Vec<String> = args
        .all
        .iter()
        .filter(|arg| *arg != "--json")
        .cloned()
        .collect();
    match run_plugin_command(&clean, json_output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            print_plugin_error(&message, json_output);
            ExitCode::FAILURE
        }
    }
}

/// The plugins in effect, with what could not be read said on stderr.
fn configured() -> Vec<PluginConfig> {
    let (configured, warnings) = plugins::configured();
    for warning in warnings {
        eprintln!("⚠ {warning}");
    }
    configured
}

fn run_plugin_command(args: &[String], json_output: bool) -> Result<(), String> {
    let sub = args.get(1).map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" => {
            print_plugin_list(&configured(), json_output);
            Ok(())
        }
        "add" | "install" => add_plugin_command(args, json_output),
        "show" => {
            let Some(name) = args.get(2) else {
                return Err("plugin show requires a plugin name".to_string());
            };
            let plugins = configured();
            let plugin = plugins::find_plugin(&plugins, name)
                .ok_or_else(|| format!("Plugin '{}' is not configured", name))?;
            print_plugin(plugin, json_output);
            Ok(())
        }
        "run" => {
            let Some(name) = args.get(2) else {
                return Err("plugin run requires a plugin name".to_string());
            };
            let Some(request_type) = args.get(3) else {
                return Err("plugin run requires a request type".to_string());
            };
            let payload = parse_run_payload(args)?;
            let plugins = configured();
            let plugin = plugins::find_plugin(&plugins, name)
                .ok_or_else(|| format!("Plugin '{}' is not configured", name))?;
            if is_core_plugin_entrypoint(request_type) {
                return Err(format!(
                    "plugin run cannot invoke core plugin entrypoint '{}'; use the dedicated vexdom command path",
                    request_type
                ));
            }
            let capability = if plugins::plugin_has_capability(plugin, request_type) {
                request_type.as_str()
            } else {
                CAPABILITY_COMMAND_RUN
            };
            let response = plugins::invoke_plugin(
                plugin,
                request_type,
                capability,
                payload,
                plugins::COMMAND_TIMEOUT,
                true,
            )
            .map_err(|e| format!("PLUGIN_FAILED: {}", e))?;
            println!("{}", response);
            Ok(())
        }
        _ => Err(format!(
            "Unknown plugin subcommand '{}'. Valid options: add, list, show, run",
            sub
        )),
    }
}

fn parse_run_payload(args: &[String]) -> Result<Value, String> {
    let mut payload = json!({});
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "--payload" => {
                let Some(raw) = args.get(i + 1) else {
                    return Err("plugin run --payload requires JSON".to_string());
                };
                payload = serde_json::from_str(raw)
                    .map_err(|e| format!("plugin run --payload is invalid JSON: {}", e))?;
                i += 1;
            }
            other => {
                return Err(format!("unknown flag '{}' for plugin run", other));
            }
        }
        i += 1;
    }
    Ok(payload)
}

fn is_core_plugin_entrypoint(entrypoint: &str) -> bool {
    matches!(
        entrypoint,
        CAPABILITY_CREDENTIAL_READ | TYPE_PLUGIN_MANIFEST | "credential.resolve"
    )
}

fn print_plugin_list(plugins: &[PluginConfig], json_output: bool) {
    if json_output {
        println!("{}", json!({ "plugins": plugins }));
        return;
    }
    if plugins.is_empty() {
        println!("No plugins configured");
        return;
    }
    for plugin in plugins {
        let capabilities = if plugin.capabilities.is_empty() {
            "(no capabilities)".to_string()
        } else {
            plugin.capabilities.join(", ")
        };
        println!("{}  {}", plugin.name, capabilities);
    }
}

fn print_plugin(plugin: &PluginConfig, json_output: bool) {
    if json_output {
        println!("{}", json!({ "plugin": plugin }));
        return;
    }
    println!("Name: {}", plugin.name);
    println!("Command: {}", plugin.command);
    if !plugin.args.is_empty() {
        println!("Args: {}", plugin.args.join(" "));
    }
    if plugin.capabilities.is_empty() {
        println!("Capabilities: (none)");
    } else {
        println!("Capabilities: {}", plugin.capabilities.join(", "));
    }
}

fn print_plugin_error(message: &str, json_output: bool) {
    if json_output {
        println!("{}", json!({ "success": false, "error": message }));
    } else {
        eprintln!("{}", message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn plugin_run_rejects_core_entrypoints() {
        assert!(is_core_plugin_entrypoint(CAPABILITY_CREDENTIAL_READ));
        assert!(is_core_plugin_entrypoint("credential.resolve"));
        assert!(is_core_plugin_entrypoint(TYPE_PLUGIN_MANIFEST));
        assert!(!is_core_plugin_entrypoint(CAPABILITY_COMMAND_RUN));
        assert!(!is_core_plugin_entrypoint("captcha.solve"));
    }

    #[test]
    fn plugin_add_source_detection_matches_reference_shape() {
        let npm = parse_plugin_source("vexdom-plugin-captcha").unwrap();
        assert_eq!(npm.kind, PluginSourceKind::Npm);
        assert_eq!(npm.install_spec, "vexdom-plugin-captcha");
        assert_eq!(npm.source, "npm:vexdom-plugin-captcha");
        assert_eq!(derive_plugin_name(&npm), "captcha");

        let scoped = parse_plugin_source("@acme/vexdom-plugin-vault@1.2.0").unwrap();
        assert_eq!(scoped.kind, PluginSourceKind::Npm);
        assert_eq!(scoped.install_spec, "@acme/vexdom-plugin-vault@1.2.0");
        assert_eq!(scoped.source, "npm:@acme/vexdom-plugin-vault@1.2.0");
        assert_eq!(derive_plugin_name(&scoped), "vault");

        let github = parse_plugin_source("acme/vexdom-plugin-browserbox").unwrap();
        assert_eq!(github.kind, PluginSourceKind::Github);
        assert_eq!(github.install_spec, "github:acme/vexdom-plugin-browserbox");
        assert_eq!(github.source, "github:acme/vexdom-plugin-browserbox");
        assert_eq!(derive_plugin_name(&github), "browserbox");

        assert!(parse_plugin_source("  ").is_err());
    }

    #[test]
    fn a_local_file_is_its_own_command() {
        let dir = std::env::temp_dir().join(format!("vexdom-cli-plugin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("fake-plugin");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        let source = parse_plugin_source(&file.to_string_lossy()).unwrap();
        assert_eq!(source.kind, PluginSourceKind::Path);
        assert_eq!(derive_plugin_name(&source), "fake");
        let options =
            parse_plugin_add_args(&words("plugin add x --capability command.run")).unwrap();
        let plugin = build_plugin_config_from_add(&source, &options, None).unwrap();
        assert_eq!(plugin.command, source.install_spec);
        assert!(plugin.args.is_empty());
        assert!(plugin.source.unwrap().starts_with("path:"));
        assert!(
            parse_plugin_source("./no-such-plugin-file")
                .unwrap_err()
                .starts_with("plugin add: no file at ")
        );
    }

    #[test]
    fn plugin_add_builds_config_from_manual_capabilities() {
        let args = words(
            "plugin add vexdom-plugin-captcha --capability command.run --capability captcha.solve --no-manifest",
        );
        let options = parse_plugin_add_args(&args).unwrap();
        assert!(options.no_manifest);
        let source = parse_plugin_source(&options.reference).unwrap();
        let plugin = build_plugin_config_from_add(&source, &options, None).unwrap();

        assert_eq!(plugin.name, "captcha");
        assert_eq!(plugin.command, "npx");
        assert_eq!(plugin.args, ["-y", "vexdom-plugin-captcha"]);
        assert_eq!(plugin.capabilities, ["command.run", "captcha.solve"]);
        assert_eq!(plugin.source.as_deref(), Some("npm:vexdom-plugin-captcha"));
    }

    #[test]
    fn plugin_add_takes_its_name_and_capabilities_from_the_manifest() {
        let options = parse_plugin_add_args(&words("plugin add @acme/vault")).unwrap();
        let source = parse_plugin_source(&options.reference).unwrap();
        let manifest = PluginManifest {
            name: Some("onepassword".into()),
            capabilities: vec![CAPABILITY_CREDENTIAL_READ.into()],
            description: None,
        };
        let plugin = build_plugin_config_from_add(&source, &options, Some(manifest)).unwrap();
        assert_eq!(plugin.name, "onepassword");
        assert_eq!(plugin.capabilities, [CAPABILITY_CREDENTIAL_READ]);
        assert!(
            build_plugin_config_from_add(&source, &options, None)
                .unwrap_err()
                .starts_with("Plugin did not declare capabilities")
        );
    }

    #[test]
    fn plugin_add_arguments_are_checked() {
        assert_eq!(
            parse_plugin_add_args(&words("plugin add")).err().unwrap(),
            "plugin add requires a package or repository reference"
        );
        assert_eq!(
            parse_plugin_add_args(&words("plugin add a b"))
                .err()
                .unwrap(),
            "unexpected argument 'b' for plugin add"
        );
        assert_eq!(
            parse_plugin_add_args(&words("plugin add a --bogus"))
                .err()
                .unwrap(),
            "unknown flag '--bogus' for plugin add"
        );
        assert!(
            parse_plugin_add_args(&words("plugin add a --project"))
                .err()
                .unwrap()
                .contains("~/.vexdom/config.json")
        );
        assert!(parse_plugin_add_args(&words("plugin add a --global --capability x")).is_ok());
        assert_eq!(
            validate_plugin_name("has space").unwrap_err(),
            "plugin name cannot contain whitespace or ':'"
        );
    }

    #[test]
    fn plugin_config_upsert_replaces_existing_plugin() {
        let entries = vec![
            json!({"name": "captcha", "command": "old", "capabilities": ["command.run"]}),
            json!({"name": "other", "command": "x", "extra": true}),
        ];
        let plugin = PluginConfig {
            name: "captcha".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "vexdom-plugin-captcha".to_string()],
            capabilities: vec!["command.run".to_string(), "captcha.solve".to_string()],
            source: Some("npm:vexdom-plugin-captcha".to_string()),
        };

        let merged = upsert_entries(entries, &plugin).unwrap();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0]["extra"], true);
        assert_eq!(merged[1]["name"], "captcha");
        assert_eq!(merged[1]["command"], "npx");
        assert_eq!(merged[1]["source"], "npm:vexdom-plugin-captcha");
        assert_eq!(merged[1]["capabilities"][1], "captcha.solve");
    }

    #[test]
    fn plugin_run_reads_one_json_payload() {
        assert_eq!(
            parse_run_payload(&words(r#"plugin run x echo --payload {"a":1}"#)).unwrap(),
            json!({"a": 1})
        );
        assert_eq!(
            parse_run_payload(&words("plugin run x echo")).unwrap(),
            json!({})
        );
        assert!(
            parse_run_payload(&words("plugin run x echo --payload"))
                .unwrap_err()
                .ends_with("requires JSON")
        );
        assert_eq!(
            parse_run_payload(&words("plugin run x echo --wat")).unwrap_err(),
            "unknown flag '--wat' for plugin run"
        );
    }
}
