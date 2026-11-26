//! The active agent CLI's own skills (`~/.claude/skills`), listed for the panel's picker and
//! attached to a message by the id the listing gave them (`agent-skills.ts`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use vexdom_core::config::AgentConfig;
use vexdom_core::log;
use vexdom_core::skills::{SKILL_FILE, split_front_matter};

use super::abort::{RunError, run_error};
use super::runners::runner_of;
use vexdom_pure::text::{flatten, js_trim, locale_compare, utf16_prefix};

const MAX_SKILLS: usize = 100;
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 200;
const MAX_SKILL_BYTES: u64 = 48 * 1024;
const TTL: Duration = Duration::from_secs(30);

/// `AgentSkillMeta` (`src/lib/actions/protocol.ts`): what the picker shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentSkillMeta {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// A skill attached to a message, read at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkill {
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone)]
struct KnownSkill {
    meta: AgentSkillMeta,
    agent: String,
    path: PathBuf,
}

struct Listing {
    at: Instant,
    agent: String,
    dirs: Vec<PathBuf>,
    skills: Vec<KnownSkill>,
}

/// The last listing, and every id ever handed to the extension: an id resolves here or nowhere,
/// and an id from the wire never becomes a path.
struct Catalog {
    listed: Option<Listing>,
    known: BTreeMap<String, KnownSkill>,
}

static CATALOG: Mutex<Catalog> = Mutex::new(Catalog::new());

fn catalog() -> MutexGuard<'static, Catalog> {
    CATALOG
        .lock()
        .expect("no listing panics while it holds the catalog")
}

/// The active agent's skills, sorted by name; a listing is reused for half a minute unless
/// `refresh`.
pub fn agent_skills(config: &AgentConfig, refresh: bool) -> Vec<AgentSkillMeta> {
    let dirs = runner_of(&config.agent).skill_dirs();
    catalog().list(&config.agent, &dirs, refresh)
}

/// The skill a listed id names, re-read from disk without its front matter.
pub fn resolve_agent_skill(id: &str, config: &AgentConfig) -> Result<AgentSkill, RunError> {
    let dirs = runner_of(&config.agent).skill_dirs();
    catalog().resolve(id, &config.agent, &dirs)
}

impl Catalog {
    const fn new() -> Self {
        Self {
            listed: None,
            known: BTreeMap::new(),
        }
    }

    fn list(&mut self, agent: &str, dirs: &[PathBuf], refresh: bool) -> Vec<AgentSkillMeta> {
        if !refresh
            && let Some(listed) = &self.listed
            && listed.agent == agent
            && listed.dirs == dirs
            && listed.at.elapsed() < TTL
        {
            return listed
                .skills
                .iter()
                .map(|skill| skill.meta.clone())
                .collect();
        }

        let mut found = Vec::new();
        for dir in dirs {
            scan(dir, agent, &mut found);
        }
        found.sort_by(|a, b| locale_compare(&a.meta.name, &b.meta.name));
        let total = found.len();
        found.truncate(MAX_SKILLS);
        if total > found.len() {
            log::log(&format!(
                "agent skills: listing {MAX_SKILLS} of {total} found for {agent}"
            ));
        }

        self.known.retain(|_, entry| entry.agent != agent);
        for skill in &found {
            self.known.insert(skill.meta.id.clone(), skill.clone());
        }
        let metas = found.iter().map(|skill| skill.meta.clone()).collect();
        self.listed = Some(Listing {
            at: Instant::now(),
            agent: agent.to_owned(),
            dirs: dirs.to_vec(),
            skills: found,
        });
        metas
    }

    fn resolve(&mut self, id: &str, agent: &str, dirs: &[PathBuf]) -> Result<AgentSkill, RunError> {
        let well_formed =
            id.len() == 16 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !well_formed {
            return Err(unknown());
        }
        if !self.known.contains_key(id) {
            self.list(agent, dirs, true);
        }
        let Some(entry) = self.known.get(id).filter(|entry| entry.agent == agent) else {
            return Err(unknown());
        };
        if !dirs
            .iter()
            .any(|dir| entry.path != *dir && entry.path.starts_with(dir))
        {
            return Err(unknown());
        }
        let Ok(stats) = std::fs::metadata(&entry.path) else {
            return Err(unknown());
        };
        if !stats.is_file() {
            return Err(unknown());
        }
        if stats.len() > MAX_SKILL_BYTES {
            return Err(run_error(
                "SKILL_UNKNOWN",
                format!(
                    "The skill \"{}\" grew past {} KB, so it was not attached.",
                    entry.meta.name,
                    MAX_SKILL_BYTES / 1024
                ),
            ));
        }
        let Ok(raw) = std::fs::read(&entry.path) else {
            return Err(unknown());
        };
        let (_, body) = split_front_matter(&String::from_utf8_lossy(&raw));
        let body = js_trim(&body);
        if body.is_empty() {
            return Err(unknown());
        }
        Ok(AgentSkill {
            name: entry.meta.name.clone(),
            body: body.to_owned(),
        })
    }
}

fn unknown() -> RunError {
    run_error(
        "SKILL_UNKNOWN",
        "That agent skill is not in the catalog any more — reopen the picker and choose again.",
    )
}

fn scan(dir: &Path, agent: &str, out: &mut Vec<KnownSkill>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str().filter(|name| !name.starts_with('.')) else {
            continue;
        };
        // The joined path is stat'ed rather than the entry trusted, so symlinked skill folders
        // count too.
        let path = if name.ends_with(".md") {
            dir.join(name)
        } else {
            dir.join(name).join(SKILL_FILE)
        };
        let Ok(stats) = std::fs::metadata(&path) else {
            continue;
        };
        if !stats.is_file() || stats.len() > MAX_SKILL_BYTES {
            continue;
        }
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let (fields, body) = split_front_matter(&String::from_utf8_lossy(&raw));
        if js_trim(&body).is_empty() {
            continue;
        }
        let named = unquote(fields.get("name").map(String::as_str));
        let fallback = name.strip_suffix(".md").unwrap_or(name);
        let name = clean(if named.is_empty() { fallback } else { named }, MAX_NAME);
        if name.is_empty() {
            continue;
        }
        let description = clean(
            unquote(fields.get("description").map(String::as_str)),
            MAX_DESCRIPTION,
        );
        out.push(KnownSkill {
            meta: AgentSkillMeta {
                id: id_of(&path),
                name,
                description,
            },
            agent: agent.to_owned(),
            path,
        });
    }
}

/// Titles cross the wire into the panel; control characters and length are stripped here, not
/// there.
fn clean(value: &str, max: usize) -> String {
    let spaced: String = value
        .chars()
        .map(|c| {
            if c <= '\u{1f}' || c == '\u{7f}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    utf16_prefix(&flatten(&spaced), max).to_owned()
}

/// A value wrapped in matching quotes, unwrapped.
fn unquote(value: Option<&str>) -> &str {
    let value = value.unwrap_or_default();
    for quote in ['"', '\''] {
        if value.len() >= 2
            && let Some(inner) = value
                .strip_prefix(quote)
                .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

fn id_of(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let mut id = String::with_capacity(16);
    for byte in &digest[..8] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_support::scratch;

    struct Skills {
        dirs: Vec<PathBuf>,
        catalog: Catalog,
    }

    impl Skills {
        fn new(home: &Path) -> Self {
            Self {
                dirs: vec![home.join(".claude").join("skills")],
                catalog: Catalog::new(),
            }
        }

        fn dir(&self) -> &Path {
            &self.dirs[0]
        }

        fn listed(&mut self) -> Vec<AgentSkillMeta> {
            self.catalog.list("claude", &self.dirs, true)
        }

        fn id_of(&mut self, name: &str) -> String {
            self.listed()
                .into_iter()
                .find(|skill| skill.name == name)
                .map(|skill| skill.id)
                .unwrap_or_default()
        }

        fn resolve(&mut self, id: &str) -> Result<AgentSkill, RunError> {
            self.catalog.resolve(id, "claude", &self.dirs)
        }
    }

    fn skill_file(path: &Path, body: &str, fields: &[(&str, &str)]) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let front: Vec<String> = fields
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect();
        let text = if front.is_empty() {
            body.to_owned()
        } else {
            format!("---\n{}\n---\n\n{body}", front.join("\n"))
        };
        std::fs::write(path, text).unwrap();
        path.to_path_buf()
    }

    fn grow(path: &Path, bytes: u64) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(bytes)
            .unwrap();
    }

    fn is_id(id: &str) -> bool {
        id.len() == 16
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }

    #[test]
    fn a_flat_file_and_a_folder_with_skill_md_both_count_sorted_each_with_an_opaque_id() {
        let home = scratch("agent-skills-list");
        let mut skills = Skills::new(home.path());
        skill_file(
            &skills.dir().join("review.md"),
            "Review the diff.",
            &[("description", "Reviews code")],
        );
        skill_file(
            &skills.dir().join("deploy").join("SKILL.md"),
            "Ship it.",
            &[
                ("name", "deploy-app"),
                ("description", "\"Deploys the app\""),
            ],
        );
        let listed = skills.listed();
        assert!(listed.iter().all(|skill| is_id(&skill.id)), "{listed:?}");
        let shown: Vec<(&str, &str)> = listed
            .iter()
            .map(|skill| (skill.name.as_str(), skill.description.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("deploy-app", "Deploys the app"),
                ("review", "Reviews code")
            ]
        );
    }

    #[test]
    fn hidden_files_skills_with_no_body_and_skills_over_48_kb_are_left_out() {
        let home = scratch("agent-skills-skip");
        let mut skills = Skills::new(home.path());
        skill_file(&skills.dir().join(".draft.md"), "Not yet.", &[]);
        skill_file(
            &skills.dir().join("empty.md"),
            "   ",
            &[("description", "nothing below")],
        );
        grow(
            &skill_file(&skills.dir().join("huge.md"), "x", &[]),
            49 * 1024,
        );
        skill_file(&skills.dir().join("kept.md"), "Do the thing.", &[]);
        let names: Vec<String> = skills
            .listed()
            .into_iter()
            .map(|skill| skill.name)
            .collect();
        assert_eq!(names, ["kept"]);
    }

    #[test]
    fn a_name_or_description_is_cleaned_and_cut_to_length_before_it_reaches_the_panel() {
        let home = scratch("agent-skills-clean");
        let mut skills = Skills::new(home.path());
        let long = "d".repeat(300);
        skill_file(
            &skills.dir().join("x.md"),
            "Body.",
            &[("name", "bad\u{7}name"), ("description", &long)],
        );
        let listed = skills.listed();
        assert_eq!(listed[0].name, "bad name");
        assert_eq!(listed[0].description, "d".repeat(200));
    }

    #[cfg(unix)]
    #[test]
    fn a_skill_folder_that_is_a_symlink_counts() {
        let home = scratch("agent-skills-link");
        let mut skills = Skills::new(home.path());
        let elsewhere = home.path().join("elsewhere").join("linked");
        skill_file(&elsewhere.join("SKILL.md"), "Linked body.", &[]);
        std::fs::create_dir_all(skills.dir()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, skills.dir().join("linked")).unwrap();
        let names: Vec<String> = skills
            .listed()
            .into_iter()
            .map(|skill| skill.name)
            .collect();
        assert_eq!(names, ["linked"]);
    }

    #[test]
    fn a_new_skill_appears_on_refresh_and_the_listing_is_reused_otherwise() {
        let home = scratch("agent-skills-cache");
        let mut skills = Skills::new(home.path());
        skill_file(&skills.dir().join("first.md"), "One.", &[]);
        skills.listed();
        skill_file(&skills.dir().join("second.md"), "Two.", &[]);
        let reused = skills.catalog.list("claude", &skills.dirs, false).len();
        assert_eq!([reused, skills.listed().len()], [1, 2]);
    }

    #[test]
    fn only_the_first_hundred_are_listed() {
        let home = scratch("agent-skills-hundred");
        let mut skills = Skills::new(home.path());
        for i in 0..105 {
            skill_file(&skills.dir().join(format!("skill-{i:03}.md")), "Body.", &[]);
        }
        let listed = skills.listed();
        assert_eq!(listed.len(), 100);
        assert_eq!(listed[0].name, "skill-000");
        let logged = std::fs::read_to_string(vexdom_core::paths::log_path()).unwrap();
        assert!(logged.contains("agent skills: listing 100 of 105 found for claude"));
    }

    #[test]
    fn an_id_from_the_listing_resolves_to_the_skill_re_read_without_its_front_matter() {
        let home = scratch("agent-skills-resolve");
        let mut skills = Skills::new(home.path());
        let path = skill_file(
            &skills.dir().join("review.md"),
            "Old body.",
            &[("description", "Reviews code")],
        );
        let id = skills.id_of("review");
        skill_file(&path, "New body.", &[("description", "Reviews code")]);
        assert_eq!(
            skills.resolve(&id),
            Ok(AgentSkill {
                name: "review".into(),
                body: "New body.".into(),
            })
        );
    }

    #[test]
    fn ids_the_daemon_never_issued_never_reach_the_disk() {
        let home = scratch("agent-skills-forged");
        let mut skills = Skills::new(home.path());
        assert_eq!(
            skills.resolve("../../.ssh/id_rsa").unwrap_err().code,
            "SKILL_UNKNOWN"
        );
        assert_eq!(
            skills.resolve("0123456789abcdef"),
            Err(unknown()),
            "a well-formed id nobody was given is unknown"
        );
    }

    #[test]
    fn a_skill_deleted_emptied_or_turned_into_a_folder_since_it_was_listed_is_unknown() {
        let home = scratch("agent-skills-gone");
        let mut skills = Skills::new(home.path());
        let gone = skill_file(&skills.dir().join("gone.md"), "Body.", &[]);
        let hollow = skill_file(&skills.dir().join("hollow.md"), "Body.", &[]);
        let shape = skill_file(&skills.dir().join("shape.md"), "Body.", &[]);
        let ids = [
            skills.id_of("gone"),
            skills.id_of("hollow"),
            skills.id_of("shape"),
        ];
        std::fs::remove_file(gone).unwrap();
        std::fs::write(&hollow, "---\nname: hollow\n---\n\n").unwrap();
        std::fs::remove_file(&shape).unwrap();
        std::fs::create_dir(&shape).unwrap();
        for id in ids {
            assert_eq!(skills.resolve(&id), Err(unknown()), "{id}");
        }
    }

    #[test]
    fn a_skill_that_grew_past_48_kb_since_it_was_listed_says_so() {
        let home = scratch("agent-skills-grown");
        let mut skills = Skills::new(home.path());
        let path = skill_file(&skills.dir().join("grown.md"), "Body.", &[]);
        let id = skills.id_of("grown");
        grow(&path, 49 * 1024);
        assert_eq!(
            skills.resolve(&id),
            Err(run_error(
                "SKILL_UNKNOWN",
                "The skill \"grown\" grew past 48 KB, so it was not attached."
            ))
        );
    }

    #[test]
    fn quotes_are_unwrapped_only_when_they_match() {
        assert_eq!(unquote(Some("\"x\"")), "x");
        assert_eq!(unquote(Some("'x'")), "x");
        assert_eq!(unquote(Some("\"x'")), "\"x'");
        assert_eq!(unquote(Some("\"")), "\"");
        assert_eq!(unquote(None), "");
    }
}
