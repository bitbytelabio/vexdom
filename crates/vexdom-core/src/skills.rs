//! Skills: markdown guidance with front matter (`src/lib/skills/format.ts`,
//! `src/daemon/agent/{skills,skill-store}.ts`). Bundled skills are embedded; user skills live in
//! `<state>/skills` and uploaded ones in `skillsDir` (default `~/vexdom/skills`), each shadowing
//! the one before by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::downloads::uploaded_skills_dir;
use crate::{log, paths};

pub const SKILL_FILE: &str = "SKILL.md";
pub const SCRIPTING_SKILL: &str = "page-scripting";
pub const MAX_BODY_BYTES: usize = 32 * 1024;
const MAX_DOMAINS: usize = 20;
const MAX_TRIGGERS: usize = 30;
const MAX_DESCRIPTION: usize = 300;
const MAX_SKILL_FILES: usize = 50;
const MAX_NAME_LENGTH: usize = 48;

/// The skills that ship inside the binary: `assets/skills/<name>/SKILL.md`.
pub const BUNDLED: &[(&str, &str)] = &[
    ("a-eye", include_str!("../assets/skills/a-eye/SKILL.md")),
    ("auth", include_str!("../assets/skills/auth/SKILL.md")),
    (
        "browse-navigation",
        include_str!("../assets/skills/browse-navigation/SKILL.md"),
    ),
    (
        "browser-control",
        include_str!("../assets/skills/browser-control/SKILL.md"),
    ),
    ("capture", include_str!("../assets/skills/capture/SKILL.md")),
    ("captcha", include_str!("../assets/skills/captcha/SKILL.md")),
    ("code", include_str!("../assets/skills/code/SKILL.md")),
    (
        "derive-client",
        include_str!("../assets/skills/derive-client/SKILL.md"),
    ),
    (
        "devtools",
        include_str!("../assets/skills/devtools/SKILL.md"),
    ),
    ("dialogs", include_str!("../assets/skills/dialogs/SKILL.md")),
    ("dogfood", include_str!("../assets/skills/dogfood/SKILL.md")),
    (
        "downloads",
        include_str!("../assets/skills/downloads/SKILL.md"),
    ),
    ("drag", include_str!("../assets/skills/drag/SKILL.md")),
    (
        "emulation",
        include_str!("../assets/skills/emulation/SKILL.md"),
    ),
    ("flows", include_str!("../assets/skills/flows/SKILL.md")),
    ("input", include_str!("../assets/skills/input/SKILL.md")),
    (
        "monitor-progress",
        include_str!("../assets/skills/monitor-progress/SKILL.md"),
    ),
    (
        "navigation",
        include_str!("../assets/skills/navigation/SKILL.md"),
    ),
    ("network", include_str!("../assets/skills/network/SKILL.md")),
    (
        "page-diagnostics",
        include_str!("../assets/skills/page-diagnostics/SKILL.md"),
    ),
    (
        "page-research",
        include_str!("../assets/skills/page-research/SKILL.md"),
    ),
    (
        "page-scripting",
        include_str!("../assets/skills/page-scripting/SKILL.md"),
    ),
    (
        "page-state",
        include_str!("../assets/skills/page-state/SKILL.md"),
    ),
    (
        "page-storage",
        include_str!("../assets/skills/page-storage/SKILL.md"),
    ),
    (
        "page-theming",
        include_str!("../assets/skills/page-theming/SKILL.md"),
    ),
    (
        "scheduled-jobs",
        include_str!("../assets/skills/scheduled-jobs/SKILL.md"),
    ),
    (
        "site-mapper",
        include_str!("../assets/skills/site-mapper/SKILL.md"),
    ),
    ("slack", include_str!("../assets/skills/slack/SKILL.md")),
    ("vexdom", include_str!("../assets/skills/vexdom/SKILL.md")),
    ("webmcp", include_str!("../assets/skills/webmcp/SKILL.md")),
    (
        "webmcp-gen",
        include_str!("../assets/skills/webmcp-gen/SKILL.md"),
    ),
];

/// The other files of bundled skills: (skill, path inside its folder, body).
pub const BUNDLED_FILES: &[(&str, &str, &str)] = &[
    (
        "dogfood",
        "references/issue-taxonomy.md",
        include_str!("../assets/skills/dogfood/references/issue-taxonomy.md"),
    ),
    (
        "dogfood",
        "templates/report.md",
        include_str!("../assets/skills/dogfood/templates/report.md"),
    ),
    (
        "vexdom",
        "references/batch.md",
        include_str!("../assets/skills/vexdom/references/batch.md"),
    ),
    (
        "vexdom",
        "references/read.md",
        include_str!("../assets/skills/vexdom/references/read.md"),
    ),
    (
        "vexdom",
        "references/snapshot-refs.md",
        include_str!("../assets/skills/vexdom/references/snapshot-refs.md"),
    ),
];

/// Bundled skills served over MCP (prompts, `skill://` resources, the skills extension, `guide`).
/// `a-eye` and `site-mapper` drive side-panel run modes and stay there. `guide` lists them in this
/// order: the core playbook, the group playbooks, then the task playbooks.
pub const MCP_SKILLS: &[&str] = &[
    "vexdom",
    "navigation",
    "input",
    "page-storage",
    "code",
    "downloads",
    "drag",
    "network",
    "emulation",
    "dialogs",
    "capture",
    "devtools",
    "webmcp",
    "flows",
    "auth",
    "page-state",
    "captcha",
    "page-theming",
    "page-diagnostics",
    "monitor-progress",
    "scheduled-jobs",
    "browser-control",
    "browse-navigation",
    "page-research",
    "page-scripting",
    "dogfood",
    "derive-client",
    "webmcp-gen",
    "slack",
];

/// One skill as an MCP client sees it: `files[0]` is `SKILL.md` (front matter included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSkill {
    pub name: String,
    pub description: String,
    pub files: Vec<(String, &'static str)>,
}

impl McpSkill {
    /// `SKILL.md` without its front matter.
    pub fn body(&self) -> String {
        split_front_matter(self.files[0].1).1
    }

    pub fn uri(&self, file: &str) -> String {
        format!("skill://{}/{file}", self.name)
    }
}

/// The skills served over MCP, in [`MCP_SKILLS`] order.
pub fn mcp_skills() -> Vec<McpSkill> {
    MCP_SKILLS
        .iter()
        .filter_map(|name| {
            let raw = BUNDLED.iter().find(|(n, _)| n == name)?.1;
            let (fields, _) = split_front_matter(raw);
            let mut files = vec![(SKILL_FILE.to_owned(), raw)];
            files.extend(
                BUNDLED_FILES
                    .iter()
                    .filter(|(skill, ..)| skill == name)
                    .map(|(_, path, body)| ((*path).to_owned(), *body)),
            );
            Some(McpSkill {
                name: (*name).to_owned(),
                description: fields.get("description").cloned().unwrap_or_default(),
                files,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub is_default: bool,
    /// `general` | `site-exploration`.
    pub category: String,
    pub domains: Vec<String>,
    /// `bundled` | `user` | `uploaded`.
    pub source: String,
    /// `authored` | `generated`.
    pub provenance: String,
    pub body: String,
    /// Where it was read from; `None` for bundled skills.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

static FRONT_MATTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^---[ \t]*\r?\n((?s:.)*?)\r?\n---[ \t]*\r?\n?").expect("front matter compiles")
});
static SKILL_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,47}$").expect("skill name compiles"));
static DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$")
        .expect("domain compiles")
});
static EXPLICIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@([a-zA-Z0-9][a-zA-Z0-9-]*)\s+").expect("explicit compiles"));

pub fn is_skill_name(name: &str) -> bool {
    SKILL_NAME.is_match(name)
}

/// Front matter `key: value` lines and the body after them.
pub fn split_front_matter(raw: &str) -> (HashMap<String, String>, String) {
    let Some(found) = FRONT_MATTER.captures(raw) else {
        return (HashMap::new(), raw.to_owned());
    };
    let mut fields = HashMap::new();
    for line in found[1].split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(separator) = line.find(':').filter(|at| *at > 0) {
            fields.insert(
                line[..separator].trim().to_owned(),
                line[separator + 1..].trim().to_owned(),
            );
        }
    }
    (fields, raw[found[0].len()..].to_owned())
}

/// `[a, 'b', "c"]` or `a, b`.
pub fn parse_list(value: Option<&str>) -> Vec<String> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let inner = value.strip_prefix('[').unwrap_or(value);
    let inner = inner.strip_suffix(']').unwrap_or(inner);
    inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            let item = item.strip_prefix(['\'', '"']).unwrap_or(item);
            item.strip_suffix(['\'', '"']).unwrap_or(item).to_owned()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

pub fn parse_category(value: Option<&str>) -> &'static str {
    if value.map(str::trim) == Some("site-exploration") {
        "site-exploration"
    } else {
        "general"
    }
}

fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn hostname_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

pub fn normalize_domain(input: &str) -> String {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return String::new();
    }
    let from_url = if trimmed.contains("://") {
        hostname_of(&trimmed)
    } else {
        trimmed
    };
    let stripped = from_url.trim_matches('/');
    let first = stripped.split('/').next().unwrap_or("");
    first
        .split(':')
        .next()
        .unwrap_or("")
        .trim_matches('.')
        .to_owned()
}

pub fn is_domain(value: &str) -> bool {
    value == "localhost" || (value.len() <= 253 && DOMAIN.is_match(value))
}

/// Domains a host falls under, longest first.
pub fn matched_domains(host: &str, domains: &[String]) -> Vec<String> {
    let lower = host.to_lowercase();
    if lower.is_empty() {
        return Vec::new();
    }
    let mut matched: Vec<String> = domains
        .iter()
        .filter(|domain| lower == **domain || lower.ends_with(&format!(".{domain}")))
        .cloned()
        .collect();
    matched.sort_by_key(|domain| std::cmp::Reverse(domain.len()));
    matched
}

fn fingerprint(value: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for unit in value.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x01000193);
    }
    let mut n = hash;
    let mut digits = Vec::new();
    loop {
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits)
        .expect("ascii")
        .chars()
        .take(5)
        .collect()
}

/// A skill name for a mapped host (`skillNameForHost`).
pub fn skill_name_for_host(host: &str, disambiguate: bool) -> String {
    let domain = normalize_domain(host);
    let domain = domain.strip_prefix("www.").unwrap_or(&domain);
    let mut base = String::new();
    for c in domain.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_matches('-').to_owned();
    if base.is_empty() {
        return format!("site-{}", fingerprint(host));
    }
    let suffix = format!("-{}", fingerprint(host));
    let truncated = base.len() > MAX_NAME_LENGTH - suffix.len();
    if !truncated && !disambiguate {
        return if base.len() <= MAX_NAME_LENGTH {
            base
        } else {
            base[..MAX_NAME_LENGTH].trim_end_matches('-').to_owned()
        };
    }
    let room = MAX_NAME_LENGTH - suffix.len();
    format!(
        "{}{suffix}",
        base[..room.min(base.len())].trim_end_matches('-')
    )
}

/// A validated skill draft (`SkillDraft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub category: String,
    pub domains: Vec<String>,
    pub triggers: Vec<String>,
    pub body: String,
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `validateSkillDraft` over the JSON the side panel sends.
pub fn validate_skill_draft(draft: &Value, reserved: &[&str]) -> Result<SkillDraft, String> {
    let name = draft
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if name.is_empty() {
        return Err("Give the skill a name.".into());
    }
    if !is_skill_name(&name) {
        return Err("Names use lowercase letters, digits and hyphens only, starting with a letter or digit (max 48).".into());
    }
    if reserved
        .iter()
        .any(|reserved| reserved.to_lowercase() == name)
    {
        return Err(format!(
            "\"{name}\" is the name of a built-in skill. Pick another."
        ));
    }
    let description = vexdom_pure::text::utf16_prefix(
        &flatten(
            draft
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        MAX_DESCRIPTION,
    )
    .to_owned();
    let category = match draft.get("category").and_then(Value::as_str) {
        Some(category @ ("general" | "site-exploration")) => category.to_owned(),
        other => {
            return Err(format!(
                "Unknown category \"{}\".",
                other.map_or_else(|| "undefined".to_owned(), str::to_owned)
            ));
        }
    };
    let domains = dedupe(
        strings(draft.get("domains"))
            .iter()
            .map(|domain| normalize_domain(domain))
            .filter(|domain| !domain.is_empty())
            .collect(),
    );
    if category == "site-exploration" {
        if domains.is_empty() {
            return Err(
                "A site-exploration skill needs at least one domain, like acme.com.".into(),
            );
        }
        if domains.len() > MAX_DOMAINS {
            return Err(format!("At most {MAX_DOMAINS} domains."));
        }
        for domain in &domains {
            if domain.contains('*') {
                return Err(format!(
                    "\"{domain}\" — wildcards are not supported; subdomains match automatically."
                ));
            }
            if !is_domain(domain) {
                return Err(format!(
                    "\"{domain}\" is not a domain. Use something like acme.com."
                ));
            }
        }
    }
    let triggers = if category == "site-exploration" {
        Vec::new()
    } else {
        dedupe(
            strings(draft.get("triggers"))
                .iter()
                .map(|trigger| flatten(trigger).to_lowercase())
                .filter(|trigger| !trigger.is_empty())
                .collect(),
        )
    };
    if triggers.len() > MAX_TRIGGERS {
        return Err(format!("At most {MAX_TRIGGERS} triggers."));
    }
    for trigger in &triggers {
        let length = trigger.encode_utf16().count();
        if !(3..=60).contains(&length) {
            return Err(format!("Trigger \"{trigger}\" must be 3–60 characters."));
        }
    }
    let raw_body = draft
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .replace("\r\n", "\n");
    let body = split_front_matter(&raw_body).1.trim().to_owned();
    if body.is_empty() {
        return Err(
            "The skill has no instructions — add some markdown below the front matter.".into(),
        );
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "The skill is larger than {} KB. Trim it down.",
            MAX_BODY_BYTES / 1024
        ));
    }
    Ok(SkillDraft {
        name,
        description,
        category,
        domains,
        triggers,
        body,
    })
}

pub fn serialize_skill_file(draft: &SkillDraft) -> String {
    let mut lines = vec![format!("name: {}", flatten(&draft.name))];
    if !draft.description.is_empty() {
        lines.push(format!("description: {}", flatten(&draft.description)));
    }
    if draft.category != "general" {
        lines.push(format!("category: {}", flatten(&draft.category)));
    }
    if !draft.domains.is_empty() {
        lines.push(format!(
            "domains: [{}]",
            draft
                .domains
                .iter()
                .map(|d| flatten(d))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !draft.triggers.is_empty() {
        lines.push(format!(
            "triggers: [{}]",
            draft
                .triggers
                .iter()
                .map(|t| flatten(t))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    format!(
        "---\n{}\n---\n\n{}\n",
        lines.join("\n"),
        draft.body.replace("\r\n", "\n").trim()
    )
}

fn parse_skill(raw: &str, fallback_name: &str, source: &str, path: Option<PathBuf>) -> Skill {
    let (fields, body) = split_front_matter(raw);
    let category = parse_category(fields.get("category").map(String::as_str));
    Skill {
        name: fields
            .get("name")
            .filter(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| fallback_name.to_owned()),
        description: fields.get("description").cloned().unwrap_or_default(),
        triggers: parse_list(fields.get("triggers").map(String::as_str))
            .iter()
            .map(|t| t.to_lowercase())
            .collect(),
        is_default: category == "general"
            && fields.get("default").map(String::as_str) == Some("true"),
        category: category.to_owned(),
        domains: if category == "site-exploration" {
            parse_list(fields.get("domains").map(String::as_str))
                .iter()
                .map(|d| d.to_lowercase())
                .collect()
        } else {
            Vec::new()
        },
        source: source.to_owned(),
        provenance: if fields.get("provenance").map(String::as_str) == Some("generated")
            && source == "uploaded"
        {
            "generated".into()
        } else {
            "authored".into()
        },
        body,
        path,
    }
}

pub fn user_skills_dir() -> PathBuf {
    paths::state_dir().join("skills")
}

pub fn user_skill_path(name: &str) -> PathBuf {
    user_skills_dir().join(format!("{name}.md"))
}

pub fn existing_user_skill(name: &str) -> bool {
    user_skill_path(name).exists() || user_skills_dir().join(name).join(SKILL_FILE).exists()
}

/// The folders skills are read from, in shadowing order (bundled ones are embedded).
pub fn skill_dir_names() -> Vec<String> {
    vec![
        "(bundled)".into(),
        user_skills_dir().to_string_lossy().into_owned(),
        uploaded_skills_dir().to_string_lossy().into_owned(),
    ]
}

fn read_dir(dir: &Path, source: &str) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && name.ends_with(".md") {
            files.push(name);
        } else if kind.is_dir() && dir.join(&name).join(SKILL_FILE).exists() {
            directories.push(name);
        }
    }
    files.sort();
    directories.sort();
    let flat: Vec<String> = files
        .iter()
        .map(|file| file.trim_end_matches(".md").to_owned())
        .collect();
    let mut skills = Vec::new();
    let mut push = |path: PathBuf, fallback: &str| match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let skill = parse_skill(&raw, fallback, source, Some(path.clone()));
            if skill.body.trim().is_empty() {
                log::log(&format!("skill {} has no body; ignoring", path.display()));
            } else {
                if skill.category == "site-exploration" && skill.domains.is_empty() {
                    log::log(&format!(
                        "skill {} is site-exploration with no domains; it will only apply via @{}",
                        skill.name, skill.name
                    ));
                }
                skills.push(skill);
            }
        }
        Err(error) => log::log_detail(
            &format!("failed to read skill {}", path.display()),
            &format!("Error: {error}"),
        ),
    };
    for name in &directories {
        if flat.contains(name) {
            log::log(&format!(
                "skill \"{name}\" exists as both {name}.md and {name}/{SKILL_FILE} in {}; using the file",
                dir.display()
            ));
            continue;
        }
        push(dir.join(name).join(SKILL_FILE), name);
    }
    for file in &files {
        push(dir.join(file), file.trim_end_matches(".md"));
    }
    skills
}

pub fn bundled_skills() -> Vec<Skill> {
    BUNDLED
        .iter()
        .map(|(name, raw)| parse_skill(raw, name, "bundled", None))
        .collect()
}

pub fn bundled_skill_names() -> Vec<String> {
    bundled_skills()
        .into_iter()
        .map(|skill| skill.name)
        .collect()
}

/// Every skill, a later folder shadowing an earlier one by name.
pub fn load_skills() -> Vec<Skill> {
    let mut by_name: Vec<Skill> = Vec::new();
    let layers = [
        bundled_skills(),
        read_dir(&user_skills_dir(), "user"),
        read_dir(&uploaded_skills_dir(), "uploaded"),
    ];
    for skill in layers.into_iter().flatten() {
        match by_name.iter_mut().find(|known| known.name == skill.name) {
            Some(known) => *known = skill,
            None => by_name.push(skill),
        }
    }
    by_name
}

/// The base skill a run follows, the overlays riding along, and the instruction without `@name`.
#[derive(Debug, Clone)]
pub struct RoutedSkill {
    pub base: Skill,
    pub overlays: Vec<Skill>,
    pub text: String,
}

/// `routeSkill`: `url` is the tab's, `live_tools` the composer's switch.
pub fn route_skill(
    skills: &[Skill],
    instruction: &str,
    url: Option<&str>,
    live_tools: bool,
) -> Option<RoutedSkill> {
    let first = skills.first()?;
    let bases: Vec<&Skill> = skills
        .iter()
        .filter(|skill| skill.category == "general" && skill.name != SCRIPTING_SKILL)
        .collect();
    let fallback = bases
        .iter()
        .find(|skill| skill.is_default)
        .or(bases.first())
        .copied()
        .unwrap_or(first);
    let mut text = instruction.to_owned();
    let mut forced: Vec<Skill> = Vec::new();
    if let Some(explicit) = EXPLICIT.captures(instruction) {
        let wanted = explicit[1].to_lowercase();
        if let Some(named) = skills
            .iter()
            .find(|skill| skill.name.to_lowercase() == wanted)
        {
            text = instruction[explicit[0].len()..].to_owned();
            if named.category == "general" {
                return Some(RoutedSkill {
                    base: named.clone(),
                    overlays: overlays_for(skills, url, live_tools, &[]),
                    text,
                });
            }
            forced.push(named.clone());
        }
    }
    let haystack = text.to_lowercase();
    let mut best = fallback;
    let mut best_score = 0;
    for skill in &bases {
        let score = skill
            .triggers
            .iter()
            .filter(|trigger| haystack.contains(trigger.as_str()))
            .count();
        if score > best_score {
            best = skill;
            best_score = score;
        }
    }
    Some(RoutedSkill {
        base: best.clone(),
        overlays: overlays_for(skills, url, live_tools, &forced),
        text,
    })
}

fn overlays_for(
    skills: &[Skill],
    url: Option<&str>,
    live_tools: bool,
    forced: &[Skill],
) -> Vec<Skill> {
    let host = url.map(hostname_of).unwrap_or_default();
    let mut matched: Vec<(Skill, String)> = if host.is_empty() {
        Vec::new()
    } else {
        skills
            .iter()
            .filter(|skill| {
                skill.category == "site-exploration" && !forced.iter().any(|f| f.name == skill.name)
            })
            .filter_map(|skill| {
                let best = matched_domains(&host, &skill.domains).into_iter().next()?;
                Some((skill.clone(), best))
            })
            .collect()
    };
    matched.sort_by_key(|(_, domain)| std::cmp::Reverse(domain.len()));
    let mut out = forced.to_vec();
    out.extend(matched.into_iter().map(|(skill, _)| skill));
    if live_tools {
        out.extend(
            skills
                .iter()
                .filter(|skill| {
                    skill.name == SCRIPTING_SKILL && !forced.iter().any(|f| f.name == skill.name)
                })
                .cloned(),
        );
    }
    out
}

fn path_in(dir: &Path, name: &str) -> Option<PathBuf> {
    is_skill_name(name).then(|| dir.join(format!("{name}.md")))
}

fn count_skills(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                !name.starts_with('.')
                    && entry.file_type().is_ok_and(|kind| {
                        if kind.is_file() {
                            name.ends_with(".md")
                        } else {
                            kind.is_dir() && dir.join(&name).join(SKILL_FILE).exists()
                        }
                    })
            })
            .count()
    })
}

/// `Err((code, message))` in `ActionResult` terms.
pub type Failure = (String, String);

fn failure<T>(code: &str, message: String) -> Result<T, Failure> {
    Err((code.to_owned(), message))
}

/// `saveSkill`: `{name, path, replaced}`.
pub fn save_skill(draft: &Value) -> Result<Value, Failure> {
    let reserved = bundled_skill_names();
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let skill = validate_skill_draft(draft, &reserved)
        .or_else(|message| failure("INVALID_INPUT", message))?;
    if existing_user_skill(&skill.name) {
        return failure(
            "NAME_TAKEN",
            format!(
                "\"{}\" already exists at {}. Rename this skill, or remove that file first.",
                skill.name,
                user_skill_path(&skill.name).display()
            ),
        );
    }
    let dir = uploaded_skills_dir();
    let Some(path) = path_in(&dir, &skill.name) else {
        return failure(
            "INVALID_INPUT",
            format!("\"{}\" is not a usable skill name.", skill.name),
        );
    };
    if dir.join(&skill.name).join(SKILL_FILE).exists() {
        return failure(
            "NAME_TAKEN",
            format!(
                "\"{}\" is a mapped site. Remove that map first, or give this skill another name.",
                skill.name
            ),
        );
    }
    let replaced = path.exists();
    if !replaced && count_skills(&dir) >= MAX_SKILL_FILES {
        return failure(
            "TOO_MANY_SKILLS",
            format!("There are already {MAX_SKILL_FILES} uploaded skills. Remove one first."),
        );
    }
    let temp = path.with_extension("md.tmp");
    let written = paths::write_private(&temp, serialize_skill_file(&skill).as_bytes())
        .and_then(|()| std::fs::rename(&temp, &path));
    if let Err(error) = written {
        log::log_detail(
            &format!("failed to save skill {}", skill.name),
            &format!("Error: {error}"),
        );
        return failure(
            "WRITE_FAILED",
            format!("Could not write {}: Error: {error}", path.display()),
        );
    }
    log::log(&format!(
        "{} skill {} ({}) at {}",
        if replaced { "replaced" } else { "saved" },
        skill.name,
        skill.category,
        path.display()
    ));
    Ok(json!({"name": skill.name, "path": path.to_string_lossy(), "replaced": replaced}))
}

pub fn delete_skill(name: &str) -> Result<Value, Failure> {
    let dir = uploaded_skills_dir();
    let Some(path) = path_in(&dir, name) else {
        return failure(
            "INVALID_INPUT",
            format!("\"{name}\" is not a usable skill name."),
        );
    };
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return failure(
            "WRITE_FAILED",
            format!("Could not remove {}: Error: {error}", path.display()),
        );
    }
    log::log(&format!("removed skill {name}"));
    Ok(json!({ "name": name }))
}

pub fn delete_site_map(name: &str) -> Result<Value, Failure> {
    let dir = uploaded_skills_dir();
    if path_in(&dir, name).is_none() {
        return failure(
            "INVALID_INPUT",
            format!("\"{name}\" is not a usable skill name."),
        );
    }
    let map_dir = dir.join(name);
    if !map_dir.join(SKILL_FILE).exists() {
        return failure("NOT_FOUND", format!("No mapped site called \"{name}\"."));
    }
    if let Err(error) = std::fs::remove_dir_all(&map_dir) {
        return failure(
            "WRITE_FAILED",
            format!("Could not remove {}: Error: {error}", map_dir.display()),
        );
    }
    log::log(&format!("removed site map {name}"));
    Ok(json!({ "name": name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;

    #[test]
    fn bundled_load_with_a_default() {
        let skills = bundled_skills();
        assert_eq!(skills.len(), BUNDLED.len());
        let default = skills.iter().find(|skill| skill.is_default).unwrap();
        assert_eq!(default.name, "browser-control");
        let routed = route_skill(&skills, "take a screenshot of this", None, false).unwrap();
        assert_eq!(routed.base.name, "browser-control");
        let forced = route_skill(&skills, "@captcha get past it", None, true).unwrap();
        assert_eq!(forced.text, "get past it");
        assert!(
            forced
                .overlays
                .iter()
                .any(|skill| skill.name == SCRIPTING_SKILL)
                || forced.base.name == "captcha"
        );
    }

    #[test]
    fn the_mcp_playbooks_are_bundled_and_well_formed() {
        let playbooks = mcp_skills();
        assert_eq!(
            playbooks.len(),
            MCP_SKILLS.len(),
            "every MCP skill is bundled"
        );
        let old_names =
            Regex::new(r"page_[a-zA-Z]+|vexdom_(openSkill|use|snapshot|read|batch|status)")
                .unwrap();
        for skill in &playbooks {
            let (fields, body) = split_front_matter(skill.files[0].1);
            assert_eq!(fields.get("name"), Some(&skill.name));
            assert!(
                !skill.description.is_empty() && skill.description.len() <= MAX_DESCRIPTION,
                "{} needs a description of at most {MAX_DESCRIPTION} bytes",
                skill.name
            );
            assert!(!skill.description.contains("Firefox"), "{}", skill.name);
            assert!(body.len() <= MAX_BODY_BYTES, "{} is too long", skill.name);
            for (path, text) in &skill.files {
                assert!(
                    !old_names.is_match(text),
                    "{}/{path} names a tool the MCP server no longer has",
                    skill.name
                );
            }
            // `guide` resolves actions before playbooks, so a playbook named like one is unreachable.
            assert_eq!(
                crate::tools::resolve_action(&skill.name),
                None,
                "{}",
                skill.name
            );
        }
        for (skill, ..) in BUNDLED_FILES {
            assert!(
                MCP_SKILLS.contains(skill),
                "{skill} has files but is not served"
            );
        }
    }

    #[test]
    fn every_group_playbook_is_served() {
        use crate::tools::{bridge, chromium, firefox};
        for manifest in [chromium(), firefox(), bridge()] {
            for group in manifest.group_names() {
                let listed = manifest
                    .tool_group(group)
                    .expect("every listed group has an index and skills");
                for name in listed.skills {
                    assert!(
                        MCP_SKILLS.contains(&name.as_str()),
                        "group {group} names {name}, which MCP does not serve"
                    );
                }
            }
        }
    }

    #[test]
    fn the_new_playbooks_are_never_routed_by_trigger() {
        let skills = bundled_skills();
        for name in ["vexdom", "network", "dogfood", "slack", "page-state"] {
            let skill = skills
                .iter()
                .find(|skill| skill.name == name)
                .expect("the playbook is bundled");
            assert!(
                skill.triggers.is_empty(),
                "{name} would hijack side-panel routing"
            );
        }
        let routed = route_skill(&skills, "check my slack unreads", None, false).unwrap();
        assert_ne!(routed.base.name, "slack");
    }

    #[test]
    fn format_helpers() {
        let (fields, body) = split_front_matter("---\nname: x\ntriggers: [a, 'b']\n---\nbody");
        assert_eq!(fields["name"], "x");
        assert_eq!(
            parse_list(fields.get("triggers").map(String::as_str)),
            ["a", "b"]
        );
        assert_eq!(body, "body");
        assert_eq!(
            normalize_domain("https://WWW.Acme.com:8080/x"),
            "www.acme.com"
        );
        assert_eq!(skill_name_for_host("www.acme.com", false), "acme-com");
        assert_eq!(
            matched_domains("app.acme.com", &["acme.com".into(), "app.acme.com".into()]),
            ["app.acme.com", "acme.com"]
        );
    }

    #[test]
    fn store_round_trip() {
        let _home = scratch_home("skills");
        crate::paths::write_private_json(
            &crate::config::config_path(),
            &json!({"skillsDir": crate::paths::state_dir().join("up").to_string_lossy()}),
        )
        .unwrap();
        let draft = json!({"name": "My-Skill", "description": "d", "category": "general", "domains": [], "triggers": ["do it"], "body": "Steps"});
        let saved = save_skill(&draft).unwrap();
        assert_eq!(saved["replaced"], false);
        assert!(
            std::fs::read_to_string(saved["path"].as_str().unwrap())
                .unwrap()
                .starts_with(
                    "---\nname: my-skill\ndescription: d\ntriggers: [do it]\n---\n\nSteps\n"
                )
        );
        assert!(
            load_skills()
                .iter()
                .any(|skill| skill.name == "my-skill" && skill.source == "uploaded")
        );
        assert_eq!(
            save_skill(&json!({"name": "captcha", "category": "general", "body": "x"}))
                .unwrap_err()
                .1,
            "\"captcha\" is the name of a built-in skill. Pick another."
        );
        assert!(delete_skill("my-skill").is_ok());
        assert_eq!(delete_site_map("nope").unwrap_err().0, "NOT_FOUND");
    }
}
