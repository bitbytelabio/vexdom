//! What the window shows, read from the Bridge's JSON (`Control` replies and `vexdom … --json`).
//! Kept apart from GPUI so it is tested without a window.

use std::collections::HashSet;

use serde_json::Value;

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The indicator lamp beside the power switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lamp {
    /// No Bridge answers.
    Off,
    /// The Bridge runs; no browser is on it.
    Waiting,
    Connected,
}

/// What the window says first: the lamp, its caption and the one line under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headline {
    pub lamp: Lamp,
    pub caption: &'static str,
    pub title: String,
}

/// The headline: whether the Bridge runs and which browser is on it (`Control.status`).
pub fn headline(status: Option<&Value>) -> Headline {
    let Some(status) = status else {
        return Headline {
            lamp: Lamp::Off,
            caption: "Bridge stopped",
            title: "Vexdom is off".to_owned(),
        };
    };
    if status.get("connected") != Some(&Value::Bool(true)) {
        return Headline {
            lamp: Lamp::Waiting,
            caption: "Bridge running",
            title: "Waiting for your browser".to_owned(),
        };
    }
    let count = status
        .get("connectedBrowsers")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let browser = str_of(status, "browser");
    let title = match (count, browser.is_empty()) {
        (2.., _) => format!("{count} browsers are connected"),
        (_, false) => format!("{browser} is connected"),
        (_, true) => "Your browser is connected".to_owned(),
    };
    Headline {
        lamp: Lamp::Connected,
        caption: "Bridge running",
        title,
    }
}

/// The facts beside the switch: label, value.
pub fn facts(
    status: Option<&Value>,
    agent: Option<&AgentCard>,
    sessions: &[SessionRow],
) -> Vec<(&'static str, String)> {
    let Some(status) = status else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    if let Some(port) = status.get("port").and_then(Value::as_u64) {
        facts.push(("Address", format!("127.0.0.1:{port}")));
    }
    if let Some(agent) = agent {
        let model = if agent.model.is_empty() {
            "default model"
        } else {
            agent.model.as_str()
        };
        facts.push((
            "Agent",
            if agent.ready {
                format!("Claude Code {} · {model}", short_version(&agent.detail))
            } else {
                "Claude Code is not ready".to_owned()
            },
        ));
    }
    let paired: Vec<&str> = sessions
        .iter()
        .map(|session| session.title.as_str())
        .collect();
    facts.push((
        "Paired",
        if paired.is_empty() {
            "No browser yet".to_owned()
        } else {
            paired.join(", ")
        },
    ));
    facts
}

/// A browser asking to pair (`Control.pairRequests`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRequest {
    pub id: String,
    /// "Chrome wants to connect to Vexdom".
    pub name: String,
    /// What the label field starts with.
    pub suggested_label: String,
}

/// `[request]` or `{requests: [request]}`.
pub fn pair_requests(value: &Value) -> Vec<PairRequest> {
    let list = value
        .as_array()
        .or_else(|| value.get("requests").and_then(Value::as_array));
    list.map(|requests| {
        requests
            .iter()
            .filter(|request| !str_of(request, "id").is_empty())
            .map(|request| {
                let browser = str_of(request, "browser");
                PairRequest {
                    id: str_of(request, "id").to_owned(),
                    name: if browser.is_empty() {
                        "A browser".to_owned()
                    } else {
                        browser.to_owned()
                    },
                    suggested_label: browser.to_owned(),
                }
            })
            .collect()
    })
    .unwrap_or_default()
}

/// A notice the Bridge queued for this computer (`Control.takeNotices`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: String,
    pub title: String,
    pub body: String,
}

/// `[notice]` or `{notices: [notice]}`.
pub fn notices(value: &Value) -> Vec<Notice> {
    let list = value
        .as_array()
        .or_else(|| value.get("notices").and_then(Value::as_array));
    list.map(|notices| {
        notices
            .iter()
            .filter(|notice| !str_of(notice, "id").is_empty())
            .map(|notice| Notice {
                id: str_of(notice, "id").to_owned(),
                title: str_of(notice, "title").to_owned(),
                body: str_of(notice, "body").to_owned(),
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The notices shown so far, newest first; one already shown is not shown (or notified) again.
#[derive(Debug, Default)]
pub struct NoticeBook {
    seen: HashSet<String>,
    pub shown: Vec<Notice>,
}

impl NoticeBook {
    /// Adds what is new and returns it, for the OS notification.
    pub fn take(&mut self, incoming: Vec<Notice>) -> Vec<Notice> {
        let fresh: Vec<Notice> = incoming
            .into_iter()
            .filter(|notice| self.seen.insert(notice.id.clone()))
            .collect();
        for notice in fresh.iter().rev() {
            self.shown.insert(0, notice.clone());
        }
        fresh
    }
}

/// A paired browser (`Control.sessions`, as `bridge.rs` turns it into JSON).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub detail: String,
    /// The browser's name as the extension reports it ("Chrome").
    pub browser: String,
    pub connected: bool,
    /// Where its extension came from: `unpacked` (a folder `vexdom setup` keeps) or `firefox`.
    pub source: String,
    pub extension_version: String,
}

pub fn sessions(values: &[Value]) -> Vec<SessionRow> {
    values
        .iter()
        .map(|session| {
            let label = str_of(session, "label");
            let browser = str_of(session, "browser");
            let title = match (label.is_empty(), browser.is_empty()) {
                (false, _) => label.to_owned(),
                (true, false) => browser.to_owned(),
                (true, true) => "A browser".to_owned(),
            };
            let seen = str_of(session, "lastSeenAt");
            let connected = session.get("connected") == Some(&Value::Bool(true));
            let detail = if connected {
                format!("{browser} · connected")
            } else if seen.is_empty() {
                format!("{browser} · never seen")
            } else {
                format!("{browser} · seen {seen}")
            };
            SessionRow {
                id: str_of(session, "id").to_owned(),
                title,
                detail: detail.trim_start_matches(" · ").to_owned(),
                browser: browser.to_owned(),
                connected,
                // An older Bridge leaves it out; the CLI reads that as unpacked too.
                source: match str_of(session, "source") {
                    "" => "unpacked".to_owned(),
                    source => source.to_owned(),
                },
                extension_version: str_of(session, "extensionVersion").to_owned(),
            }
        })
        .collect()
}

/// One sentence for what `vexdom setup --browser <id> --json` did, with the step the user still
/// has to take.
pub fn setup_summary(value: &Value) -> String {
    let Some(chosen) = value.get("chosen").filter(|chosen| chosen.is_object()) else {
        return "Vexdom is set up.".to_owned();
    };
    let id = str_of(chosen, "id");
    let row = value
        .get("browsers")
        .and_then(Value::as_array)
        .and_then(|rows| rows.iter().find(|row| str_of(row, "id") == id));
    let label = row.map(|row| str_of(row, "label")).unwrap_or(id);
    let flag = |key: &str| chosen.get(key) == Some(&Value::Bool(true));
    let mut sentence = if flag("connected") {
        format!("{label} is already connected.")
    } else if flag("unpacked") {
        match value.get("extensionDir").and_then(Value::as_str) {
            Some(dir) => format!(
                "In {label}, turn on Developer mode and load {dir} as an unpacked extension."
            ),
            None => format!("Load the unpacked extension in {label}."),
        }
    } else if flag("opened") {
        format!("Opened the extension's page in {label}. Add it, then allow it here.")
    } else {
        format!("Vexdom is set up for {label}.")
    };
    let steps = row
        .and_then(|row| row.get("steps"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for step in steps {
        sentence.push(' ');
        sentence.push_str(step);
    }
    sentence
}

/// "2.1.332" out of `claude --version`'s "2.1.332 (Claude Code)".
pub fn short_version(detail: &str) -> &str {
    detail.split_whitespace().next().unwrap_or("")
}

/// One row of `vexdom browsers --json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRow {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub connected: bool,
    pub sessions: usize,
    /// How it gets the extension: `unpacked` (every Chromium browser) or `firefox`.
    pub source: String,
    /// `chrome://extensions`, `about:addons`, …
    pub extensions_page: String,
}

/// The installed browsers and any with a pairing, in the CLI's order.
pub fn browser_rows(value: &Value) -> Vec<BrowserRow> {
    value
        .get("browsers")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| BrowserRow {
                    id: str_of(row, "id").to_owned(),
                    label: str_of(row, "label").to_owned(),
                    installed: row.get("installed") == Some(&Value::Bool(true)),
                    connected: row.get("connected") == Some(&Value::Bool(true)),
                    sessions: row
                        .get("sessions")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                    source: str_of(row, "source").to_owned(),
                    extensions_page: str_of(row, "extensionsPage").to_owned(),
                })
                .filter(|row| row.installed || row.sessions > 0)
                .collect()
        })
        .unwrap_or_default()
}

/// Claude Code as `Control.agent` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub ready: bool,
    /// The version when ready, else what is wrong.
    pub detail: String,
    /// The model set in config.json; empty is the CLI's own default.
    pub model: String,
    pub models: Vec<String>,
    /// Why it is not ready (`AGENT_MISSING`, `AGENT_UNUSABLE`); empty when ready.
    pub problem: String,
    /// What the Bridge says fixes it, when not ready.
    pub fix: String,
}

/// The problem code that means Claude Code is not installed.
pub const AGENT_MISSING: &str = "AGENT_MISSING";

pub fn agent_card(state: &Value) -> Option<AgentCard> {
    let runner = state
        .get("runners")
        .and_then(Value::as_array)?
        .iter()
        .find(|runner| str_of(runner, "kind") == "claude")?;
    let ready = runner.get("ready") == Some(&Value::Bool(true));
    let problem = runner.get("problem");
    let detail = if ready {
        str_of(runner, "version").to_owned()
    } else {
        let message = problem
            .map(|problem| match problem {
                Value::String(text) => text.clone(),
                other => str_of(other, "message").to_owned(),
            })
            .unwrap_or_default();
        if message.is_empty() {
            format!("`{}` was not found on PATH", str_of(runner, "bin"))
        } else {
            message
        }
    };
    // Not ready with no code given: the Bridge found no program at all.
    let code = match problem.map(|problem| str_of(problem, "code")) {
        _ if ready => "",
        Some(code) if !code.is_empty() => code,
        _ => AGENT_MISSING,
    };
    let models = runner
        .get("models")
        .and_then(|models| models.get("ids"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Some(AgentCard {
        ready,
        detail,
        model: str_of(runner, "model").to_owned(),
        models,
        problem: code.to_owned(),
        fix: problem
            .filter(|_| !ready)
            .map(|problem| str_of(problem, "fix").to_owned())
            .unwrap_or_default(),
    })
}

/// An action name as the user reads it: `page.submitForm` → `submitForm`.
fn short_action(action: &str) -> &str {
    ["page.", "bridge.", "vexdom."]
        .iter()
        .find_map(|prefix| action.strip_prefix(prefix))
        .unwrap_or(action)
}

/// The unpacked extension folder `vexdom setup` keeps current (`vexdom browsers --json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unpacked {
    pub dir: String,
    /// The build the folder holds; `None` until `vexdom setup` has written it.
    pub version: Option<String>,
}

pub fn unpacked(value: &Value) -> Unpacked {
    let unpacked = value.get("unpacked").unwrap_or(&Value::Null);
    Unpacked {
        dir: str_of(unpacked, "dir").to_owned(),
        version: Some(str_of(unpacked, "version"))
            .filter(|version| !version.is_empty())
            .map(str::to_owned),
    }
}

/// Whether a browser still runs an older build of the unpacked extension than its folder holds,
/// so the extension has to be reloaded there (`vexdom doctor`'s `extension.version`).
pub fn reload_needed(session: &SessionRow, unpacked: &Unpacked) -> bool {
    session.source == "unpacked"
        && unpacked
            .version
            .as_deref()
            .is_some_and(|version| version != session.extension_version)
}

/// The app `open -a` opens for a browser id (the CLI's `BROWSERS`).
pub fn mac_app(id: &str) -> Option<&'static str> {
    Some(match id {
        "chrome" => "Google Chrome",
        "edge" => "Microsoft Edge",
        "brave" => "Brave Browser",
        "arc" => "Arc",
        "vivaldi" => "Vivaldi",
        "opera" => "Opera",
        "chromium" => "Chromium",
        "firefox" => "Firefox",
        _ => return None,
    })
}

/// What is left to do once the unpacked folder's path is on the clipboard.
pub fn unpacked_steps(label: &str, page: &str) -> String {
    format!(
        "The extension's folder is copied. In {label}, open {page}, turn on Developer mode and click Load unpacked; in the folder picker press ⇧⌘G, paste and press Return."
    )
}

/// The ways a new browser pairs (`Control.pairing`): value, label.
pub const PAIRING_MODES: [(&str, &str); 3] = [("auto", "Auto"), ("ask", "Ask"), ("code", "Code")];

/// The pairing mode in one line (the CLI's `pairing_line`).
pub fn pairing_line(mode: &str) -> &'static str {
    match mode {
        "ask" => "A browser connects once you allow it here.",
        "code" => "A browser connects only with a code from this window.",
        _ => "A browser with Vexdom connects by itself.",
    }
}

/// A pairing code as the extension asks for it: `ABCD1234` → `ABCD-1234`.
pub fn pairing_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    if chars.len() == 8 {
        let (head, tail) = chars.split_at(4);
        format!(
            "{}-{}",
            head.iter().collect::<String>(),
            tail.iter().collect::<String>()
        )
    } else {
        code.to_owned()
    }
}

/// The time left until `expires_at` (ms since the epoch) as `mm:ss`, rounded up so a code that
/// still works never reads 00:00; `None` once it has run out.
pub fn countdown(expires_at: i64, now: i64) -> Option<String> {
    let left = expires_at.saturating_sub(now);
    if left <= 0 {
        return None;
    }
    let seconds = left.saturating_add(999) / 1000;
    Some(format!("{:02}:{:02}", seconds / 60, seconds % 60))
}

/// An action from an MCP client or the CLI waiting for the user (`Control.confirmations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub id: String,
    /// Without its `page.` or `bridge.` prefix.
    pub action: String,
    /// The host it acts on; empty when it acts on none.
    pub site: String,
    pub summary: String,
}

impl Confirmation {
    pub fn question(&self) -> String {
        if self.site.is_empty() {
            format!("Allow {}?", self.action)
        } else {
            format!("Allow {} on {}?", self.action, self.site)
        }
    }
}

/// `[confirmation]`, as `Control.confirmations` and `Topic::ConfirmationsChanged` list them.
pub fn confirmations(value: &Value) -> Vec<Confirmation> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| !str_of(entry, "id").is_empty())
        .map(|entry| Confirmation {
            id: str_of(entry, "id").to_owned(),
            action: short_action(str_of(entry, "action")).to_owned(),
            site: str_of(entry, "site").to_owned(),
            summary: str_of(entry, "summary").to_owned(),
        })
        .collect()
}

/// A guardrail's effects, as `setPreference` writes them, and their labels.
pub const EFFECTS: [(&str, &str); 3] = [("allow", "Allow"), ("confirm", "Ask"), ("deny", "Block")];

/// One guardrail rule on Settings → General.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailRow {
    pub id: String,
    pub title: String,
    pub reason: String,
    /// The rule's own effect.
    pub fallback: String,
    /// What applies now: the user's choice, else the fallback.
    pub effect: String,
    /// Shown, not changeable here: config.json holds it.
    pub locked: bool,
}

impl GuardrailRow {
    /// What `setPreference` writes to pick `effect`: the rule's own effect is `null`, so the
    /// rule follows its default again.
    pub fn value_for(&self, effect: &str) -> Value {
        if effect == self.fallback {
            Value::Null
        } else {
            Value::String(effect.to_owned())
        }
    }
}

/// The theme the extension shows while config.json names none (`theme.ts` `DEFAULT_THEME`).
pub const DEFAULT_THEME: &str = "neon";

/// `Control.preferences`: the browser theme and the guardrails.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    pub theme: String,
    pub rules: Vec<GuardrailRow>,
    /// Page text reaches the agent fenced off as untrusted.
    pub fence: bool,
    /// An action that asks is allowed, not refused, when nobody is there to answer.
    pub unattended_allow: bool,
}

pub fn preferences(value: &Value) -> Preferences {
    let guardrails = value.get("guardrails").unwrap_or(&Value::Null);
    let rules = guardrails
        .get("rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|rule| !str_of(rule, "id").is_empty())
        .map(|rule| {
            let fallback = str_of(rule, "fallback");
            let chosen = str_of(rule, "override");
            GuardrailRow {
                id: str_of(rule, "id").to_owned(),
                title: str_of(rule, "title").to_owned(),
                reason: str_of(rule, "reason").to_owned(),
                fallback: fallback.to_owned(),
                effect: if chosen.is_empty() { fallback } else { chosen }.to_owned(),
                locked: rule.get("locked") == Some(&Value::Bool(true)),
            }
        })
        .collect();
    Preferences {
        theme: match str_of(value, "theme") {
            "" => DEFAULT_THEME.to_owned(),
            theme => theme.to_owned(),
        },
        rules,
        fence: guardrails
            .get("fence")
            .and_then(|fence| fence.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        unattended_allow: guardrails
            .get("unattended")
            .is_some_and(|unattended| str_of(unattended, "effect") == "allow"),
    }
}

/// "Neon" for `neon`.
pub fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// One skill of `vexdom skills --json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    /// `bundled`, `user` or `uploaded`.
    pub source: String,
    pub description: String,
}

/// The sources a skill comes from, as the filter's chips offer them.
pub const SKILL_SOURCES: [&str; 3] = ["bundled", "user", "uploaded"];

pub fn skill_rows(value: &Value) -> Vec<SkillRow> {
    value
        .get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|skill| !str_of(skill, "name").is_empty())
        .map(|skill| SkillRow {
            name: str_of(skill, "name").to_owned(),
            source: str_of(skill, "source").to_owned(),
            description: str_of(skill, "description").to_owned(),
        })
        .collect()
}

/// The skills from `source` (any when `None`) whose name or description holds `text`, in any case.
pub fn filter_skills<'a>(
    rows: &'a [SkillRow],
    text: &str,
    source: Option<&str>,
) -> Vec<&'a SkillRow> {
    let needle = text.trim().to_lowercase();
    rows.iter()
        .filter(|row| source.is_none_or(|source| row.source == source))
        .filter(|row| {
            needle.is_empty()
                || row.name.to_lowercase().contains(&needle)
                || row.description.to_lowercase().contains(&needle)
        })
        .collect()
}

/// The standing approvals of one site (`vexdom approvals --json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalHost {
    pub host: String,
    pub actions: Vec<String>,
}

/// The grants, one entry per host in the order the hosts first appear.
pub fn approval_hosts(value: &Value) -> Vec<ApprovalHost> {
    let mut hosts: Vec<ApprovalHost> = Vec::new();
    for grant in value
        .get("grants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let host = str_of(grant, "host");
        let action = short_action(str_of(grant, "action")).to_owned();
        if host.is_empty() {
            continue;
        }
        match hosts.iter_mut().find(|entry| entry.host == host) {
            Some(entry) if !entry.actions.contains(&action) => entry.actions.push(action),
            Some(_) => {}
            None => hosts.push(ApprovalHost {
                host: host.to_owned(),
                actions: vec![action],
            }),
        }
    }
    hosts
}

/// A file an agent captured (`vexdom downloads --json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRow {
    pub name: String,
    /// Size, site and day.
    pub detail: String,
    pub path: String,
}

pub fn download_rows(value: &Value) -> Vec<DownloadRow> {
    value
        .get("downloads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|download| {
            let size = download.get("size").and_then(Value::as_u64).map(size_text);
            let day = str_of(download, "capturedAt").get(..10).map(str::to_owned);
            let host = Some(str_of(download, "host").to_owned());
            let detail: Vec<String> = [size, host, day]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect();
            DownloadRow {
                name: str_of(download, "name").to_owned(),
                detail: detail.join(" · "),
                path: str_of(download, "savedTo").to_owned(),
            }
        })
        .collect()
}

/// `1536` → `1.5 KB`.
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// Where the app's own update stands (About).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    /// Checked: nothing is newer than this app.
    Latest,
    Available(String),
    Downloading(String),
    Verifying(String),
    Relaunching(String),
    Failed(String),
}

/// The release asset holding Vexdom.app for this Mac.
pub fn app_asset(version: &str, target: &str) -> String {
    format!("Vexdom-{version}-{target}.zip")
}

/// A new GitHub issue with the versions already in its body.
pub fn bug_report_url(app: &str, bridge: &str, extensions: &[String], macos: &str) -> String {
    let bridge = if bridge.is_empty() { "off" } else { bridge };
    let extension = if extensions.is_empty() {
        "none".to_owned()
    } else {
        extensions.join(", ")
    };
    let macos = if macos.is_empty() { "unknown" } else { macos };
    let body = format!("vexdom app {app}, Bridge {bridge}, extension {extension}, macOS {macos}");
    format!(
        "https://github.com/nguyenthdat/vexdom/issues/new?title=&body={}",
        url_encode(&body)
    )
}

/// Percent-encodes all but RFC 3986's unreserved characters.
pub fn url_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The command that adds Vexdom to Claude Code as an MCP server, the path quoted for a shell.
pub fn mcp_command(vexdom: &str) -> String {
    let path = if vexdom
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+~".contains(c))
    {
        vexdom.to_owned()
    } else {
        format!("'{}'", vexdom.replace('\'', r"'\''"))
    };
    format!("claude mcp add vexdom -- {path} mcp")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_headline_follows_the_bridge_and_the_browser() {
        let off = headline(None);
        assert_eq!((off.lamp, off.title.as_str()), (Lamp::Off, "Vexdom is off"));
        let waiting = headline(Some(&json!({"connected": false})));
        assert_eq!(
            (waiting.lamp, waiting.title.as_str()),
            (Lamp::Waiting, "Waiting for your browser")
        );
        let one = headline(Some(
            &json!({"connected": true, "browser": "Chrome", "connectedBrowsers": 1}),
        ));
        assert_eq!(
            (one.lamp, one.title.as_str()),
            (Lamp::Connected, "Chrome is connected")
        );
        let two = headline(Some(
            &json!({"connected": true, "browser": "Chrome", "connectedBrowsers": 2}),
        ));
        assert_eq!(two.title, "2 browsers are connected");
    }

    #[test]
    fn facts_prove_the_headline() {
        let agent = AgentCard {
            ready: true,
            detail: "2.1.332 (Claude Code)".to_owned(),
            model: String::new(),
            models: Vec::new(),
            problem: String::new(),
            fix: String::new(),
        };
        let paired = sessions(&[json!({"id": "s1", "label": "Work", "browser": "Chrome"})]);
        let facts = facts(Some(&json!({"port": 8785})), Some(&agent), &paired);
        assert_eq!(
            facts,
            [
                ("Address", "127.0.0.1:8785".to_owned()),
                ("Agent", "Claude Code 2.1.332 · default model".to_owned()),
                ("Paired", "Work".to_owned()),
            ]
        );
    }

    #[test]
    fn pair_requests_name_the_browser_and_skip_what_has_no_id() {
        let requests = pair_requests(&json!({"requests": [
            {"id": "r1", "browser": "Chrome", "source": "unpacked"},
            {"id": "r2"},
            {"browser": "Edge"},
        ]}));
        assert_eq!(requests.len(), 2);
        assert_eq!(
            (
                requests[0].name.as_str(),
                requests[0].suggested_label.as_str()
            ),
            ("Chrome", "Chrome")
        );
        assert_eq!(
            (
                requests[1].name.as_str(),
                requests[1].suggested_label.as_str()
            ),
            ("A browser", "")
        );
    }

    #[test]
    fn a_notice_is_shown_once_however_often_it_arrives() {
        let mut book = NoticeBook::default();
        let first = json!([{"id": "a", "title": "Done", "body": "1"}, {"id": "b", "title": "T", "body": ""}]);
        assert_eq!(book.take(notices(&first)).len(), 2);
        let again = json!({"notices": [{"id": "b", "title": "T", "body": ""}, {"id": "c", "title": "New", "body": ""}]});
        let fresh = book.take(notices(&again));
        assert_eq!(
            fresh.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(
            book.shown.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
    }

    #[test]
    fn sessions_prefer_the_label_and_say_when_seen() {
        let rows = sessions(&[
            json!({"id": "s1", "label": "Work", "browser": "Chrome", "connected": true}),
            json!({"id": "s2", "label": "", "browser": "Firefox", "lastSeenAt": "2026-11-26T08:00:00Z"}),
        ]);
        assert_eq!(rows[0].title, "Work");
        assert_eq!(rows[0].detail, "Chrome · connected");
        assert_eq!(rows[1].title, "Firefox");
        assert_eq!(rows[1].detail, "Firefox · seen 2026-11-26T08:00:00Z");
    }

    #[test]
    fn setup_summary_names_the_step_left() {
        let unpacked = json!({"extensionDir": "/x/ext", "chosen": {"id": "chrome", "unpacked": true},
            "browsers": [{"id": "chrome", "label": "Chrome", "steps": []}]});
        assert_eq!(
            setup_summary(&unpacked),
            "In Chrome, turn on Developer mode and load /x/ext as an unpacked extension."
        );
        let store = json!({"chosen": {"id": "firefox", "opened": true},
            "browsers": [{"id": "firefox", "label": "Firefox", "steps": ["Say yes to both."]}]});
        assert_eq!(
            setup_summary(&store),
            "Opened the extension's page in Firefox. Add it, then allow it here. Say yes to both."
        );
        assert_eq!(setup_summary(&json!({"chosen": null})), "Vexdom is set up.");
    }

    #[test]
    fn browser_rows_keep_installed_or_paired_ones() {
        let rows = browser_rows(&json!({"running": true, "browsers": [
            {"id": "chrome", "label": "Chrome", "installed": true, "connected": true, "sessions": [{"id": "s"}]},
            {"id": "edge", "label": "Edge", "installed": false, "sessions": []},
            {"id": "arc", "label": "Arc", "installed": false, "sessions": [{"id": "t"}]},
        ]}));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["chrome", "arc"]
        );
        assert!(rows[0].connected && rows[0].sessions == 1);
    }

    #[test]
    fn the_agent_card_reads_claude() {
        let card = agent_card(&json!({"runners": [{"kind": "claude", "bin": "claude", "model": "sonnet",
            "ready": true, "version": "2.1.332 (Claude Code)", "models": {"ids": ["opus", "sonnet"]}}]}))
        .expect("a card");
        assert!(card.ready);
        assert_eq!(card.detail, "2.1.332 (Claude Code)");
        assert_eq!(card.models, ["opus", "sonnet"]);
        let missing =
            agent_card(&json!({"runners": [{"kind": "claude", "bin": "claude", "ready": false}]}))
                .expect("a card");
        assert_eq!(missing.detail, "`claude` was not found on PATH");
        assert_eq!(missing.problem, AGENT_MISSING);
        assert_eq!(agent_card(&json!({})), None);
        let unusable = agent_card(&json!({"runners": [{"kind": "claude", "ready": false,
            "problem": {"code": "AGENT_UNUSABLE", "message": "It expired.", "fix": "claude login"}}]}))
        .expect("a card");
        assert_eq!(
            (unusable.problem.as_str(), unusable.fix.as_str()),
            ("AGENT_UNUSABLE", "claude login")
        );
        assert!(card.problem.is_empty() && card.fix.is_empty());
    }

    #[test]
    fn a_browser_on_an_older_unpacked_build_needs_a_reload() {
        let listed = json!({"browsers": [], "unpacked": {"dir": "/u/vexdom/extension/chrome-mv3", "version": "0.9.3"}});
        let folder = unpacked(&listed);
        assert_eq!(folder.dir, "/u/vexdom/extension/chrome-mv3");
        let rows = sessions(&[
            json!({"id": "a", "browser": "Chrome", "source": "unpacked", "extensionVersion": "0.9.2"}),
            json!({"id": "b", "browser": "Chrome", "source": "unpacked", "extensionVersion": "0.9.3"}),
            json!({"id": "c", "browser": "Firefox", "source": "firefox", "extensionVersion": "0.9.2"}),
            // An older Bridge sends no source: unpacked, as the CLI reads it.
            json!({"id": "d", "browser": "Edge", "extensionVersion": "0.9.1"}),
        ]);
        let stale: Vec<&str> = rows
            .iter()
            .filter(|row| reload_needed(row, &folder))
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(stale, ["a", "d"]);
        // No folder written yet: nothing to reload from.
        let none = unpacked(&json!({"unpacked": {"dir": "/x", "version": null}}));
        assert!(rows.iter().all(|row| !reload_needed(row, &none)));
    }

    #[test]
    fn a_pairing_code_reads_in_halves_and_counts_down_to_its_end() {
        assert_eq!(pairing_code("ABCD1234"), "ABCD-1234");
        assert_eq!(pairing_code("short"), "short");
        assert_eq!(countdown(600_000, 0).as_deref(), Some("10:00"));
        assert_eq!(countdown(61_000, 0).as_deref(), Some("01:01"));
        // A part of a second left still shows a second.
        assert_eq!(countdown(1_000, 1).as_deref(), Some("00:01"));
        assert_eq!(countdown(5_000, 5_000), None);
        assert_eq!(countdown(0, 9_000), None);
    }

    #[test]
    fn confirmations_ask_about_the_action_on_its_site() {
        let list = confirmations(&json!([
            {"id": "c-1a2b3c4d", "action": "page.submitForm", "site": "example.com", "summary": "Submit the form \"Login\""},
            {"id": "c-2", "action": "bridge.flowRun", "site": null, "summary": ""},
            {"action": "page.clickElement"},
        ]));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].question(), "Allow submitForm on example.com?");
        assert_eq!(list[1].question(), "Allow flowRun?");
        assert!(confirmations(&json!({"list": []})).is_empty());
    }

    #[test]
    fn preferences_read_the_rules_and_write_a_default_as_null() {
        let prefs = preferences(&json!({"theme": "ember", "guardrails": {
            "rules": [
                {"id": "raw-html-read", "title": "Raw HTML", "reason": "r", "fallback": "deny", "override": "allow"},
                {"id": "form-submission", "title": "Forms", "reason": "f", "fallback": "allow"},
                {"id": "secret-in-url", "title": "Secrets", "reason": "s", "fallback": "deny", "locked": true},
            ],
            "fence": {"enabled": false, "overridden": true},
            "unattended": {"effect": "allow", "overridden": true},
            "hosts": [], "configPath": "/x/config.json"}}));
        assert_eq!(prefs.theme, "ember");
        assert_eq!(
            prefs
                .rules
                .iter()
                .map(|rule| (rule.effect.as_str(), rule.locked))
                .collect::<Vec<_>>(),
            [("allow", false), ("allow", false), ("deny", true)]
        );
        assert!(!prefs.fence && prefs.unattended_allow);
        assert_eq!(prefs.rules[0].value_for("deny"), Value::Null);
        assert_eq!(prefs.rules[0].value_for("confirm"), json!("confirm"));
        let empty = preferences(&json!({}));
        assert!(empty.fence && !empty.unattended_allow && empty.rules.is_empty());
        assert_eq!(empty.theme, DEFAULT_THEME);
        assert_eq!(title_case("neon"), "Neon");
    }

    #[test]
    fn skills_filter_by_text_and_source() {
        let rows = skill_rows(&json!({"skills": [
            {"name": "vexdom", "source": "bundled", "description": "Drive the browser"},
            {"name": "dogfood", "source": "bundled", "description": "Find bugs"},
            {"name": "mine", "source": "user", "description": "My BROWSER notes"},
        ]}));
        let names = |found: Vec<&SkillRow>| -> Vec<String> {
            found.into_iter().map(|row| row.name.clone()).collect()
        };
        assert_eq!(names(filter_skills(&rows, "", None)).len(), 3);
        assert_eq!(
            names(filter_skills(&rows, " browser ", None)),
            ["vexdom", "mine"]
        );
        assert_eq!(
            names(filter_skills(&rows, "browser", Some("user"))),
            ["mine"]
        );
        assert!(filter_skills(&rows, "", Some("uploaded")).is_empty());
    }

    #[test]
    fn approvals_group_by_site_and_downloads_say_what_and_where() {
        let hosts = approval_hosts(&json!({"grants": [
            {"action": "page.submitForm", "host": "a.com", "at": "2026-11-26T08:00:00Z"},
            {"action": "page.clickElement", "host": "b.com", "at": ""},
            {"action": "page.fillInput", "host": "a.com", "at": ""},
            {"action": "page.submitForm", "host": "a.com", "at": ""},
        ]}));
        assert_eq!(
            hosts,
            [
                ApprovalHost {
                    host: "a.com".to_owned(),
                    actions: vec!["submitForm".to_owned(), "fillInput".to_owned()],
                },
                ApprovalHost {
                    host: "b.com".to_owned(),
                    actions: vec!["clickElement".to_owned()],
                },
            ]
        );
        let downloads = download_rows(&json!({"dir": "/d", "downloads": [
            {"name": "report.pdf", "size": 1536, "host": "a.com", "savedTo": "/d/report.pdf", "capturedAt": "2026-11-26T08:00:00Z"},
            {"name": "x", "size": 12, "savedTo": "/d/x", "capturedAt": ""},
        ]}));
        assert_eq!(downloads[0].detail, "1.5 KB · a.com · 2026-11-26");
        assert_eq!(downloads[0].path, "/d/report.pdf");
        assert_eq!(downloads[1].detail, "12 B");
    }

    #[test]
    fn a_bug_report_carries_the_versions() {
        assert_eq!(url_encode("a b&c=ü"), "a%20b%26c%3D%C3%BC");
        assert_eq!(
            bug_report_url("0.9.2", "0.9.2", &["0.9.1".to_owned()], "27.2"),
            "https://github.com/nguyenthdat/vexdom/issues/new?title=&body=vexdom%20app%200.9.2%2C%20Bridge%200.9.2%2C%20extension%200.9.1%2C%20macOS%2027.2"
        );
        assert!(
            bug_report_url("1", "", &[], "")
                .ends_with("Bridge%20off%2C%20extension%20none%2C%20macOS%20unknown")
        );
        assert_eq!(
            app_asset("1.0.0", "aarch64-apple-darwin"),
            "Vexdom-1.0.0-aarch64-apple-darwin.zip"
        );
    }

    #[test]
    fn the_mcp_command_quotes_a_path_with_spaces() {
        assert_eq!(
            mcp_command("/Users/ada/.local/bin/vexdom"),
            "claude mcp add vexdom -- /Users/ada/.local/bin/vexdom mcp"
        );
        assert_eq!(
            mcp_command("/Volumes/My Apps/Vexdom.app/Contents/MacOS/vexdom"),
            "claude mcp add vexdom -- '/Volumes/My Apps/Vexdom.app/Contents/MacOS/vexdom' mcp"
        );
    }
}
