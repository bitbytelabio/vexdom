//! Shared helpers for the agent tests: a scratch `VEXDOM_HOME` and a `LocalSet` runtime.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

static ENV: Mutex<()> = Mutex::new(());

/// A fresh `VEXDOM_HOME` for one test; tests that touch the state folder serialize on it.
pub struct Scratch {
    home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl Scratch {
    pub fn path(&self) -> &Path {
        &self.home
    }
}

#[allow(unsafe_code)]
pub fn scratch(name: &str) -> Scratch {
    let guard = ENV.lock().unwrap_or_else(PoisonError::into_inner);
    let home = std::env::temp_dir().join(format!("vexdom-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temp folder takes a scratch home");
    // SAFETY: every agent test that reads or writes VEXDOM_HOME holds ENV.
    unsafe { std::env::set_var("VEXDOM_HOME", &home) };
    Scratch {
        home,
        _guard: guard,
    }
}

/// Runs a test body on a current-thread runtime inside a `LocalSet`, as the daemon runs.
pub fn local<F: Future<Output = ()>>(body: F) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime builds");
    tokio::task::LocalSet::new().block_on(&runtime, body);
}

/// A stand-in Claude Code for one-shots, configured as the agent. Each run notes itself in
/// `calls`, keeps its prompt in `prompt` and its arguments in `args` (each on its own line, the
/// prompt's lines among them), copies what is in the task folder to `handed/` (modes kept), runs
/// the test's own `body`, then prints `answer.json` as its `--output-format json` result.
#[cfg(unix)]
pub struct StandIn {
    home: PathBuf,
    pub config: vexdom_core::config::AgentConfig,
}

#[cfg(unix)]
impl StandIn {
    pub fn new(home: &Path, body: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let config = vexdom_core::config::read_agent_config();
        let tasks = crate::agent::runner::task_dir(&config);
        let dir = home.display();
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = --version ]; then echo '2.1.283 (Claude Code)'; exit 0; fi\n\
             echo $$ > '{dir}/pid'\n\
             echo run >> '{dir}/calls'\n\
             printf '%s' \"$2\" > '{dir}/prompt'\n\
             printf '%s\\n' \"$@\" > '{dir}/args'\n\
             mkdir -p '{dir}/handed' && cp -p '{tasks}'/* '{dir}/handed/' 2>/dev/null\n\
             {body}\n\
             cat '{dir}/answer.json'\n",
            tasks = tasks.display(),
        );
        let bin = home.join("claude");
        std::fs::write(&bin, script).expect("the stand-in can be written");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("the stand-in can be made executable");
        let mut config = config;
        config.claude.bin = bin.to_string_lossy().into_owned();
        let stand_in = Self {
            home: home.to_path_buf(),
            config,
        };
        stand_in.answer("");
        stand_in
    }

    /// What the next runs answer.
    pub fn answer(&self, text: &str) {
        let result = serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": text});
        std::fs::write(self.home.join("answer.json"), result.to_string())
            .expect("the answer can be written");
    }

    /// What the last run was asked.
    pub fn prompt(&self) -> String {
        std::fs::read_to_string(self.home.join("prompt")).unwrap_or_default()
    }

    /// The last run's arguments.
    pub fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.home.join("args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// How many times it was started for a one-shot.
    pub fn calls(&self) -> usize {
        std::fs::read_to_string(self.home.join("calls"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    /// The copy of the file a run found in the task folder, by name.
    pub fn handed(&self, name: &str) -> PathBuf {
        self.home.join("handed").join(name)
    }

    /// Waits up to five seconds for the last run's process to be gone; the `LocalSet` has to keep
    /// running meanwhile, since that is where it is stopped.
    pub async fn stopped(&self) -> bool {
        let pid: i32 = std::fs::read_to_string(self.home.join("pid"))
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
            .expect("the stand-in ran and wrote its pid");
        let pid = nix::unistd::Pid::from_raw(pid);
        for _ in 0..100 {
            if nix::sys::signal::kill(pid, None).is_err() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
}
