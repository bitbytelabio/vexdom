//! Containment for the agent CLI the daemon spawns (`guardrails/spawn.ts`): the plan is vetted
//! before `spawn`, and the environment is sealed of anything credential-shaped.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::agents;

/// `run` drives the browser; `task` is a one-shot that must not reach it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    Run,
    Task,
}

impl SpawnMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Task => "task",
        }
    }
}

pub struct SpawnPlan<'a> {
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub files: &'a [String],
}

struct Requirements {
    required: &'static [&'static str],
    deny_flag: &'static str,
    deny_tools: &'static [&'static str],
}

struct Containment {
    local_tools: &'static str,
    keeps_env: &'static [&'static str],
    federated: &'static [(&'static str, &'static [&'static str])],
    note: &'static str,
    run: Requirements,
    task: Requirements,
}

const CLAUDE: Containment = Containment {
    local_tools: "allowlist",
    keeps_env: &["ANTHROPIC_", "CLAUDE_"],
    federated: &[
        ("CLAUDE_CODE_USE_BEDROCK", &["AWS_"]),
        (
            "CLAUDE_CODE_USE_VERTEX",
            &["GOOGLE_", "GCLOUD_", "CLOUDSDK_"],
        ),
    ],
    note: "per-run tool allowlist plus an explicit deny list",
    run: Requirements {
        required: &["--strict-mcp-config", "--allowedTools"],
        deny_flag: "--disallowedTools",
        deny_tools: &[
            "Bash",
            "Edit",
            "Write",
            "NotebookEdit",
            "Glob",
            "Grep",
            "Task",
            "Read",
        ],
    },
    task: Requirements {
        required: &["--strict-mcp-config", r#"{"mcpServers":{}}"#],
        deny_flag: "--disallowedTools",
        deny_tools: &[
            "Bash",
            "Edit",
            "Write",
            "NotebookEdit",
            "Glob",
            "Grep",
            "Task",
        ],
    },
};

fn containment(kind: &str) -> &'static Containment {
    // Claude Code is the only agent kind (`agents::AGENT_KINDS`).
    let _ = kind;
    &CLAUDE
}

static FORBIDDEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u)^(?:--dangerously|--yolo$|--force$|--auto-approve$|--always-approve$|--full-auto$|--no-sandbox$|--allow-all|--sandbox=?(?:workspace-write|danger-full-access|off)$|--permission-mode=?(?:bypassPermissions|acceptEdits)$)|(?i-u:danger-full-access)|^(?-i:-y|-f)$")
        .expect("forbidden compiles")
});

/// `^key=(?!"allowed"$)` with ASCII case folding.
fn forbidden_kv(arg: &str, key: &str, allowed: &str) -> bool {
    arg.len() >= key.len()
        && arg.is_char_boundary(key.len())
        && arg[..key.len()].eq_ignore_ascii_case(key)
        && !arg[key.len()..].eq_ignore_ascii_case(allowed)
}

fn forbidden(arg: &str) -> bool {
    FORBIDDEN.is_match(arg)
        || forbidden_kv(arg, "sandbox_mode=", "\"read-only\"")
        || forbidden_kv(arg, "approval_policy=", "\"never\"")
}

fn every_value_of<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    args.iter()
        .enumerate()
        .filter(|(at, arg)| *arg == flag && at + 1 < args.len())
        .map(|(at, _)| args[at + 1].as_str())
        .collect()
}

fn variadic<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    let Some(at) = args.iter().position(|arg| arg == flag) else {
        return Vec::new();
    };
    args[at + 1..]
        .iter()
        .take_while(|arg| !arg.starts_with("--"))
        .map(String::as_str)
        .collect()
}

/// Everything wrong with a plan, as sentences. Empty means it may spawn.
pub fn vet_plan(kind: &str, mode: SpawnMode, plan: &SpawnPlan<'_>, home: &Path) -> Vec<String> {
    let label = format!("{} ({})", agents::agent_label(kind), mode.as_str());
    let rules = match mode {
        SpawnMode::Run => &containment(kind).run,
        SpawnMode::Task => &containment(kind).task,
    };
    let mut problems = Vec::new();
    for arg in rules.required {
        if !plan.args.iter().any(|have| have == arg) {
            problems.push(format!("{label} is spawned without {arg}."));
        }
    }
    let mut named = variadic(plan.args, rules.deny_flag);
    named.extend(every_value_of(plan.args, rules.deny_flag));
    let missing: Vec<&str> = rules
        .deny_tools
        .iter()
        .copied()
        .filter(|tool| !named.contains(tool))
        .collect();
    if !missing.is_empty() {
        problems.push(format!(
            "{label} does not deny {} via {}.",
            missing.join(", "),
            rules.deny_flag
        ));
    }
    let _ = plan.files;
    for arg in plan.args {
        if forbidden(arg) {
            problems.push(format!(
                "{label} is spawned with {arg}, which disables its own containment."
            ));
        }
    }
    let mut models = every_value_of(plan.args, "--model");
    models.extend(every_value_of(plan.args, "-m"));
    for model in models {
        if !agents::is_model_id(model) {
            problems.push(format!(
                "{label} is spawned with the model {}, which it could read as a flag.",
                serde_json::to_string(model).unwrap_or_default()
            ));
        }
    }
    if !within(plan.cwd, home) {
        problems.push(format!(
            "{label} would run in {}, which is outside {}.",
            plan.cwd.display(),
            home.display()
        ));
    }
    problems
}

/// One line for the daemon log.
pub fn describe_containment(kind: &str) -> String {
    let rules = containment(kind);
    format!(
        "{} containment: {} — {}",
        agents::agent_label(kind),
        rules.local_tools,
        rules.note
    )
}

static SECRET_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u)(?:^|_)(?:KEY|KEYS|TOKEN|TOKENS|SECRET|SECRETS|PASSWORD|PASSWD|CREDENTIAL|CREDENTIALS|AUTH|SESSION|COOKIE|PRIVATE|SIGNATURE)(?:$|_)")
        .expect("secret word compiles")
});

const SECRET_PREFIX: &[&str] = &[
    "AWS_",
    "AZURE_",
    "GCP_",
    "GOOGLE_",
    "GH_",
    "GITHUB_",
    "GITLAB_",
    "BITBUCKET_",
    "NPM_",
    "YARN_",
    "PYPI_",
    "CARGO_",
    "DOCKER_",
    "KUBE_",
    "HELM_",
    "STRIPE_",
    "SLACK_",
    "TWILIO_",
    "SENDGRID_",
    "SENTRY_",
    "DATADOG_",
    "PAGERDUTY_",
    "DATABASE_",
    "POSTGRES_",
    "PGPASS",
    "MYSQL_",
    "REDIS_",
    "MONGO_",
    "SUPABASE_",
    "OPENAI_",
    "ANTHROPIC_",
    "GEMINI_",
    "CLAUDE_",
    "CODEX_",
    "ANTIGRAVITY_",
    "MISTRAL_",
    "XAI_",
    "GROK_",
    "CURSOR_",
    "HF_",
    "HUGGINGFACE_",
    "QWEN_",
    "DASHSCOPE_",
    "BAILIAN_",
    "OPENCODE_",
    "DEEPSEEK_",
    "DSH_",
    "VERCEL_",
    "NETLIFY_",
    "CLOUDFLARE_",
    "FLY_",
    "HEROKU_",
    "RAILWAY_",
];

fn enabled(value: Option<&String>) -> bool {
    value.is_some_and(|value| {
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

/// The environment a run may see: the parent's minus anything credential-shaped that does not
/// belong to the agent being spawned.
pub fn seal_env(kind: &str, env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let rules = containment(kind);
    let mut keeps: Vec<&str> = rules.keeps_env.to_vec();
    for (flag, prefixes) in rules.federated {
        if enabled(env.get(*flag)) {
            keeps.extend_from_slice(prefixes);
        }
    }
    env.iter()
        .filter(|(name, _)| {
            keeps.iter().any(|prefix| name.starts_with(prefix))
                || (!SECRET_WORD.is_match(name)
                    && !SECRET_PREFIX.iter().any(|prefix| name.starts_with(prefix)))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Names dropped by [`seal_env`], for the log.
pub fn sealed_away(kind: &str, env: &BTreeMap<String, String>) -> Vec<String> {
    let sealed = seal_env(kind, env);
    env.keys()
        .filter(|name| !sealed.contains_key(*name))
        .cloned()
        .collect()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Whether `child` is `parent` or lies under it (lexically, `..` resolved).
pub fn within(child: &Path, parent: &Path) -> bool {
    normalize(child).starts_with(normalize(parent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn vets_a_run_plan() {
        let good = args(&[
            "-p",
            "--strict-mcp-config",
            "--allowedTools",
            "mcp__vexdom__*",
            "--disallowedTools",
            "Bash",
            "Edit",
            "Write",
            "NotebookEdit",
            "Glob",
            "Grep",
            "Task",
            "Read",
            "--model",
            "opus",
        ]);
        let home = Path::new("/home/u");
        let plan = SpawnPlan {
            args: &good,
            cwd: Path::new("/home/u/.vexdom/runs/1"),
            files: &[],
        };
        assert!(vet_plan("claude", SpawnMode::Run, &plan, home).is_empty());
        let bad = args(&[
            "--dangerously-skip-permissions",
            "--model",
            "-x",
            "sandbox_mode=off",
        ]);
        let plan = SpawnPlan {
            args: &bad,
            cwd: Path::new("/home/u/../etc"),
            files: &[],
        };
        let problems = vet_plan("claude", SpawnMode::Run, &plan, home);
        assert_eq!(
            problems[0],
            "Claude Code (run) is spawned without --strict-mcp-config."
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("does not deny Bash, Edit"))
        );
        assert!(problems.iter().any(|p| p == "Claude Code (run) is spawned with --dangerously-skip-permissions, which disables its own containment."));
        assert!(problems.iter().any(|p| p.contains("sandbox_mode=off")));
        assert!(problems.iter().any(|p| p == "Claude Code (run) is spawned with the model \"-x\", which it could read as a flag."));
        assert!(
            problems
                .iter()
                .any(|p| p.ends_with("which is outside /home/u."))
        );
    }

    #[test]
    fn seals_env() {
        let env: BTreeMap<String, String> = [
            ("PATH", "/bin"),
            ("AWS_SECRET_ACCESS_KEY", "x"),
            ("ANTHROPIC_API_KEY", "k"),
            ("SESSION_TOKEN", "t"),
            ("SESSIONS_DIR", "d"),
            ("GITHUB_TOKEN", "g"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let sealed = seal_env("claude", &env);
        assert_eq!(
            sealed.keys().collect::<Vec<_>>(),
            ["ANTHROPIC_API_KEY", "PATH", "SESSIONS_DIR"]
        );
        let mut bedrock = env.clone();
        bedrock.insert("CLAUDE_CODE_USE_BEDROCK".into(), "1".into());
        assert!(seal_env("claude", &bedrock).contains_key("AWS_SECRET_ACCESS_KEY"));
        assert_eq!(
            sealed_away("claude", &env),
            ["AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN", "SESSION_TOKEN"]
        );
    }
}
