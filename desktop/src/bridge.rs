//! The app's link to the Bridge, on a thread of its own: capnp clients are `!Send`, so they live on
//! a current-thread tokio runtime inside a `LocalSet` there. The window sends [`Command`]s and reads
//! [`Update`]s. Turning the Bridge on and off, adding the extension, setting the model and the
//! first-run fixes run the `vexdom` CLI itself, with the login shell's PATH (`shell.rs`), so the
//! app never holds a second copy of that logic.

use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, WeakUnboundedSender, unbounded_channel,
};
use vexdom_client::Bridge;
use vexdom_schema::Topic;
use vexdom_update::{Layout, Release};

#[cfg(target_os = "macos")]
use crate::model::mac_app;
use crate::model::{UpdateState, app_asset, setup_summary, unpacked, unpacked_steps};
use crate::preflight::{Findings, Fix, Preflight, evaluate};
use crate::{settings, shell};

/// How often a missing Bridge is looked for, and a present one asked for its status.
const POLL: Duration = Duration::from_secs(2);

/// How often the app looks for a newer release of itself.
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Where releases are published, for an app that cannot update itself.
pub const RELEASES: &str = "https://github.com/bitbytelabio/vexdom/releases";

/// This app's version: the one a release has to beat.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the app was asked to do as it opens (`settings.rs`).
#[derive(Debug, Clone, Copy)]
pub struct Launch {
    /// Start the Bridge if it is off.
    pub start_bridge: bool,
    /// A new Vexdom.app was just put in place: finish it with `vexdom setup`.
    pub finish_update: bool,
}

pub enum Command {
    Refresh,
    Start,
    Stop,
    Restart,
    Decide {
        request: String,
        allow: bool,
        label: String,
    },
    Revoke {
        session: String,
    },
    /// Unpairs every browser.
    RevokeAll,
    /// `None` is the CLI's own default.
    SetAgent {
        model: Option<String>,
    },
    /// `vexdom agent fix claude`.
    FixAgent,
    InstallFor {
        browser: String,
    },
    /// Writes the unpacked extension, copies its folder and opens the browser's extensions page.
    LoadUnpacked {
        browser: String,
    },
    OpenNotice(String),
    /// Answers an action waiting for the user's approval.
    Confirm {
        id: String,
        allow: bool,
        remember: bool,
    },
    /// Mints a pairing code.
    PairCode,
    /// `auto`, `ask` or `code`.
    SetPairing(String),
    /// Runs the first-run checks again.
    Recheck,
    /// Runs these first-run fixes in order, stopping at the first that fails, then the checks.
    Fixes(Vec<Fix>),
    Preferences,
    /// A `Control.setPreference` change: `{kind: "theme", theme}` or
    /// `{kind: "guardrail", setting, value}`.
    SetPreference(Value),
    Skills,
    Activity,
    /// Forgets the standing approvals of one host.
    ForgetApprovals(String),
    ClearDownloads,
    /// Links the bundle's `vexdom` onto the login PATH (`true`), or removes that link.
    Link(bool),
    Uninstall {
        keep_skills: bool,
    },
    CheckUpdate,
    InstallUpdate,
}

pub enum Update {
    Disconnected,
    /// The Bridge is being started as the app opens.
    Starting,
    Status(Value),
    Sessions(Vec<Value>),
    PairRequests(Value),
    Agent(Value),
    Browsers(Value),
    Notices(Value),
    Confirmations(Value),
    PairCode {
        code: String,
        /// Milliseconds since the epoch.
        expires_at: i64,
    },
    Preflight {
        checks: Preflight,
        /// Whether `npm` is on the login PATH.
        npm: bool,
    },
    Preferences(Value),
    Skills(Value),
    Activity {
        approvals: Value,
        downloads: Value,
    },
    CommandLine(CommandLine),
    /// The unpacked extension's folder, for the clipboard, and what to do with it.
    Unpacked {
        dir: String,
        said: String,
    },
    UpdateState(UpdateState),
    /// `sw_vers -productVersion`.
    MacOs(String),
    /// A new Vexdom.app replaces this one as soon as it quits.
    Quit,
    /// What a command printed, for the status line.
    Said(String),
    Error(String),
}

/// Where the `vexdom` command is, and where it is linked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLine {
    /// The `vexdom` the app runs.
    pub vexdom: Option<PathBuf>,
    /// The `vexdom` inside this app's bundle: the one "Link vexdom" links.
    pub bundled: Option<PathBuf>,
    /// The folder on the login PATH holding a link to `bundled`.
    pub linked: Option<PathBuf>,
}

/// Starts the thread. `deliver` gets every [`Update`] (on that thread); the window keeps the
/// returned sender for its [`Command`]s.
pub fn spawn(
    launch: Launch,
    deliver: impl Fn(Update) + Send + 'static,
) -> UnboundedSender<Command> {
    let (commands, received) = unbounded_channel();
    let again = commands.downgrade();
    std::thread::Builder::new()
        .name("vexdom-bridge".into())
        .spawn(move || {
            let link = Link {
                deliver: Rc::new(deliver),
                again,
                release: Rc::default(),
                updating: Rc::default(),
            };
            vexdom_client::block_on(run(received, link, launch));
        })
        .expect("the bridge thread starts");
    commands
}

/// The `vexdom` inside this app's bundle (beside its executable), when there is one.
fn bundled() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("vexdom")))
        .filter(|beside| beside.is_file())
}

/// The `vexdom` the app runs: the bundle's, else the first on the login PATH.
pub fn vexdom_path() -> Option<PathBuf> {
    bundled().or_else(|| shell::which("vexdom"))
}

fn vexdom() -> PathBuf {
    vexdom_path().unwrap_or_else(|| PathBuf::from("vexdom"))
}

fn command_line() -> CommandLine {
    let bundled = bundled();
    CommandLine {
        vexdom: vexdom_path(),
        linked: bundled
            .as_ref()
            .and_then(|target| shell::linked_in(shell::login_path(), target)),
        bundled,
    }
}

/// The last thing a failed program said, else `fallback`.
fn last_line(out: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    stderr
        .lines()
        .chain(stdout.lines())
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_owned()
}

/// Runs `program` with the login PATH; its stdout, or the last line it wrote when it failed.
async fn exec(program: impl AsRef<OsStr>, args: &[&str]) -> Result<String, String> {
    let name = program.as_ref().to_string_lossy().into_owned();
    let out = shell::command(program)
        .args(args)
        .output()
        .await
        .map_err(|error| format!("could not run {name}: {error}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(last_line(&out, &format!("{name} failed")))
    }
}

/// Runs the CLI.
async fn cli(args: &[&str]) -> Result<String, String> {
    exec(vexdom(), args).await
}

/// Runs a `--json` command and reads its JSON whatever its exit code: `vexdom doctor` exits 1 on a
/// failed check and still prints its report.
async fn cli_json(args: &[&str]) -> Result<Value, String> {
    let out = shell::command(vexdom())
        .args(args)
        .output()
        .await
        .map_err(|error| format!("could not run vexdom: {error}"))?;
    serde_json::from_slice(&out.stdout).map_err(|_| last_line(&out, "vexdom failed"))
}

/// The first line a command printed, as the status line shows it.
fn first_line(out: &str, fallback: &str) -> String {
    out.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[derive(Clone)]
struct Link {
    deliver: Rc<dyn Fn(Update)>,
    /// The thread's own command queue, for a job that ends by asking for more.
    again: WeakUnboundedSender<Command>,
    /// The newest release found, for "Update to vX".
    release: Rc<RefCell<Option<Release>>>,
    /// An update is downloading or being put in place.
    updating: Rc<Cell<bool>>,
}

impl Link {
    fn send(&self, update: Update) {
        (self.deliver)(update);
    }

    fn queue(&self, command: Command) {
        if let Some(commands) = self.again.upgrade() {
            let _ = commands.send(command);
        }
    }

    /// Our own sentence for a command that worked, else what the CLI said went wrong.
    fn done(&self, result: Result<String, String>, sentence: impl FnOnce(&str) -> String) {
        self.send(match result {
            Ok(out) => Update::Said(sentence(&out)),
            Err(error) => Update::Error(error),
        });
    }

    async fn browsers(&self) -> Option<Value> {
        match cli_json(&["browsers", "--json"]).await {
            Ok(value) => {
                self.send(Update::Browsers(value.clone()));
                Some(value)
            }
            Err(error) => {
                self.send(Update::Error(error));
                None
            }
        }
    }

    async fn load_unpacked(&self, browser: &str) {
        let args = [
            "setup",
            "--browser",
            browser,
            "--unpacked",
            "--no-pair",
            "--no-wait",
            "--json",
        ];
        let setup = match cli_json(&args).await {
            Ok(setup) => setup,
            Err(error) => return self.send(Update::Error(error)),
        };
        let listed = self.browsers().await;
        let dir = setup
            .get("extensionDir")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| listed.as_ref().map(|listed| unpacked(listed).dir))
            .filter(|dir| !dir.is_empty());
        let Some(dir) = dir else {
            return self.send(Update::Error(
                "vexdom setup did not say where it put the extension.".to_owned(),
            ));
        };
        let row = setup
            .get("browsers")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("id").and_then(Value::as_str) == Some(browser))
            });
        let field = |key: &str| {
            row.and_then(|row| row.get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let (label, page) = (field("label"), field("extensionsPage"));
        #[cfg(target_os = "macos")]
        if let Some(app) = mac_app(browser) {
            // Brings the browser forward on its extensions page; the steps say where to go if not.
            let _ = exec("/usr/bin/open", &["-a", app, &page]).await;
        }
        self.send(Update::Unpacked {
            dir,
            said: unpacked_steps(&label, &page),
        });
    }

    async fn activity(&self) {
        match (
            cli_json(&["approvals", "--json"]).await,
            cli_json(&["downloads", "--json"]).await,
        ) {
            (Ok(approvals), Ok(downloads)) => self.send(Update::Activity {
                approvals,
                downloads,
            }),
            (Err(error), _) | (_, Err(error)) => self.send(Update::Error(error)),
        }
    }

    fn link(&self, on: bool) {
        let Some(target) = bundled() else {
            return self.send(Update::Error(
                "Only the vexdom inside Vexdom.app can be linked.".to_owned(),
            ));
        };
        let login = shell::login_path();
        if on {
            let dirs = shell::link_dirs(&vexdom_core::paths::home_dir(), login);
            match shell::link(&target, &dirs) {
                Ok(dir) => self.send(Update::Said(format!(
                    "Linked vexdom into {}.",
                    dir.display()
                ))),
                Err(error) => self.send(Update::Error(error)),
            }
        } else {
            let removed = shell::unlink(&target, login);
            self.send(Update::Said(if removed == 0 {
                "vexdom was not linked.".to_owned()
            } else {
                "Unlinked vexdom.".to_owned()
            }));
        }
        self.send(Update::CommandLine(command_line()));
    }

    /// The commands that need no connection: each runs the CLI or reads the state directly.
    async fn offline(&self, command: Command) {
        match command {
            Command::Refresh => {
                let _ = self.browsers().await;
            }
            Command::Start => self.done(cli(&["start"]).await, |_| "Vexdom is on.".into()),
            Command::Stop => self.done(cli(&["stop"]).await, |_| "Vexdom is off.".into()),
            Command::Restart => {
                self.done(cli(&["restart"]).await, |_| "Vexdom restarted.".into());
            }
            Command::InstallFor { browser } => {
                // Pairing is approved in this window, so no code is minted.
                let result = cli(&["setup", "--browser", &browser, "--json", "--no-pair"]).await;
                self.done(result, |out| {
                    serde_json::from_str(out)
                        .map(|value| setup_summary(&value))
                        .unwrap_or_else(|_| "Vexdom is set up.".into())
                });
                let _ = self.browsers().await;
            }
            Command::LoadUnpacked { browser } => self.load_unpacked(&browser).await,
            Command::SetAgent { model } => {
                let mut args = vec!["agent", "model", "claude"];
                if let Some(model) = model.as_deref() {
                    args.push(model);
                }
                let name = model.as_deref().unwrap_or("Claude Code's default");
                self.done(cli(&args).await, |_| format!("Model set to {name}."));
            }
            Command::FixAgent => {
                let result = cli(&["agent", "fix", "claude"]).await;
                self.done(result, |_| "Claude Code is set up.".into());
            }
            Command::Recheck => preflight(self, None).await,
            Command::Fixes(fixes) => {
                let link = self.clone();
                tokio::task::spawn_local(async move {
                    for fix in fixes {
                        if !run_fix(&link, fix).await {
                            break;
                        }
                    }
                    link.queue(Command::Recheck);
                });
            }
            Command::Preferences => self.send(Update::Preferences(
                vexdom_core::preferences::preferences_now(),
            )),
            Command::SetPreference(change) => {
                // No Bridge to tell: config.json is the only copy, read again when one starts.
                match vexdom_core::preferences::apply_preference(&change) {
                    Ok(now) => self.send(Update::Preferences(now)),
                    Err((_, message)) => self.send(Update::Error(message)),
                }
            }
            Command::Skills => match cli_json(&["skills", "--json"]).await {
                Ok(skills) => self.send(Update::Skills(skills)),
                Err(error) => self.send(Update::Error(error)),
            },
            Command::Activity => self.activity().await,
            Command::ForgetApprovals(host) => {
                let result = cli(&["approvals", "clear", &host]).await;
                self.done(result, |out| first_line(out, "Forgot them."));
                self.activity().await;
            }
            Command::ClearDownloads => {
                let result = cli(&["downloads", "clear"]).await;
                self.done(result, |out| first_line(out, "Deleted them."));
                self.activity().await;
            }
            Command::Link(on) => self.link(on),
            Command::Uninstall { keep_skills } => {
                if let Some(target) = bundled() {
                    shell::unlink(&target, shell::login_path());
                }
                let mut args = vec!["uninstall", "--yes"];
                if keep_skills {
                    args.push("--keep-skills");
                }
                let result = cli(&args).await;
                self.done(result, |_| "Vexdom is uninstalled.".into());
                self.send(Update::CommandLine(command_line()));
                preflight(self, None).await;
            }
            Command::CheckUpdate => {
                let link = self.clone();
                tokio::task::spawn_local(async move { check_update(&link).await });
            }
            Command::InstallUpdate => {
                let link = self.clone();
                tokio::task::spawn_local(async move { install_update(&link).await });
            }
            Command::Decide { .. }
            | Command::Revoke { .. }
            | Command::RevokeAll
            | Command::OpenNotice(_)
            | Command::Confirm { .. }
            | Command::PairCode
            | Command::SetPairing(_) => {
                self.send(Update::Error("Vexdom is off.".into()));
            }
        }
    }
}

/// Runs one first-run fix; `false` when it failed.
async fn run_fix(link: &Link, fix: Fix) -> bool {
    let result = match fix {
        Fix::SetUp => {
            link.send(Update::Said("Setting up Vexdom…".into()));
            cli(&["setup", "--no-pair", "--no-open", "--no-wait", "--json"])
                .await
                .map(|_| "Vexdom is set up.".to_owned())
        }
        Fix::InstallClaude { .. } => match shell::which("npm") {
            Some(npm) => {
                link.send(Update::Said("Installing Claude Code…".into()));
                exec(npm, &["i", "-g", "@anthropic-ai/claude-code"])
                    .await
                    .map(|_| "Claude Code is installed.".to_owned())
            }
            None => Err("npm was not found on your PATH. Install Node.js, then try again.".into()),
        },
        // A page in the browser: the window opens it.
        Fix::GetChrome => return true,
    };
    let worked = result.is_ok();
    link.done(result, |sentence| sentence.to_owned());
    worked
}

/// Gathers what the first-run checks need. Claude Code is only asked about through a running
/// Bridge: `vexdom agent` would start one.
async fn preflight(link: &Link, bridge: Option<&Bridge>) {
    let vexdom = vexdom_path();
    let (doctor, browsers) = if vexdom.is_some() {
        (
            Some(cli_json(&["doctor", "--json", "--offline"]).await),
            Some(cli_json(&["browsers", "--json"]).await),
        )
    } else {
        (None, None)
    };
    let agent = match bridge {
        Some(bridge) => agent_state(bridge).await.map(Ok),
        None => None,
    };
    let npm = shell::which("npm").is_some();
    let checks = evaluate(&Findings {
        vexdom,
        doctor,
        browsers,
        agent,
        npm,
    });
    link.send(Update::Preflight { checks, npm });
}

/// Looks for a release newer than this app.
async fn check_update(link: &Link) {
    if link.updating.get() {
        return;
    }
    link.send(Update::UpdateState(UpdateState::Checking));
    let state = match vexdom_update::latest().await {
        Ok(release) if vexdom_update::newer(&release.version, VERSION) => {
            let version = release.version.clone();
            *link.release.borrow_mut() = Some(release);
            UpdateState::Available(version)
        }
        Ok(_) => UpdateState::Latest,
        Err(error) => UpdateState::Failed(error),
    };
    if !link.updating.get() {
        link.send(Update::UpdateState(state));
    }
}

/// Downloads the release's app, checks its signature and bundle, puts it beside this one and
/// asks the window to quit: a detached shell swaps it in once this process is gone and opens it,
/// and the new app finishes the update (`finish_update`).
async fn install_update(link: &Link) {
    if link.updating.replace(true) {
        return;
    }
    let state = match stage_update(link).await {
        Ok(()) => return link.send(Update::Quit),
        Err(error) => UpdateState::Failed(error),
    };
    link.updating.set(false);
    link.send(Update::UpdateState(state));
}

async fn stage_update(link: &Link) -> Result<(), String> {
    let release = link
        .release
        .borrow()
        .clone()
        .ok_or("Check for updates first.")?;
    let version = release.version.clone();
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let bundle = match vexdom_update::detect_layout(&exe) {
        Layout::App { bundle } => bundle,
        Layout::Tarball { .. } | Layout::Unmanaged(_) => {
            return Err(format!(
                "UPDATE_UNMANAGED: {} is not inside Vexdom.app, so it cannot replace itself. Get Vexdom {version} from {RELEASES}.",
                exe.display()
            ));
        }
    };
    link.send(Update::UpdateState(UpdateState::Downloading(
        version.clone(),
    )));
    let dir = std::env::temp_dir().join(format!("vexdom-update-{version}"));
    let asset = app_asset(&version, vexdom_update::target_triple());
    let zip = vexdom_update::fetch_verified(&release, &asset, &dir).await?;
    link.send(Update::UpdateState(UpdateState::Verifying(version.clone())));
    let staged = {
        let (bundle, version) = (bundle.clone(), version.clone());
        tokio::task::spawn_blocking(move || vexdom_update::stage_app(&zip, &bundle, &version))
            .await
            .map_err(|error| error.to_string())??
    };
    let _ = std::fs::remove_dir_all(&dir);
    settings::edit(|settings| settings.finish_update_on_launch = true)
        .map_err(|error| format!("Could not note the update in desktop.json: {error}"))?;
    link.send(Update::UpdateState(UpdateState::Relaunching(version)));
    vexdom_update::swap_after_exit(std::process::id(), &staged, &bundle).inspect_err(|_| {
        let _ = settings::edit(|settings| settings.finish_update_on_launch = false);
    })
}

/// The first launch of a new Vexdom.app: the Bridge and the extension folder still hold the
/// previous build until `vexdom setup` restarts and rewrites them.
async fn finish_update(link: &Link) {
    link.send(Update::Said(format!("Finishing the update to {VERSION}…")));
    let result = cli(&[
        "setup",
        "--no-pair",
        "--restart",
        "--no-open",
        "--no-wait",
        "--json",
    ])
    .await;
    if let Err(error) = settings::edit(|settings| settings.finish_update_on_launch = false) {
        link.send(Update::Error(format!(
            "Could not clear the update note in desktop.json: {error}"
        )));
    }
    link.done(result, |_| format!("Vexdom {VERSION} is in place."));
}

async fn run(mut commands: UnboundedReceiver<Command>, link: Link, launch: Launch) {
    // The login shell's PATH is asked for once, here rather than on the window's thread.
    shell::login_path();
    link.send(Update::CommandLine(command_line()));
    if let Ok(version) = exec("/usr/bin/sw_vers", &["-productVersion"]).await {
        link.send(Update::MacOs(version.trim().to_owned()));
    }
    if launch.finish_update {
        finish_update(&link).await;
    }
    let updates = link.clone();
    tokio::task::spawn_local(async move {
        loop {
            check_update(&updates).await;
            tokio::time::sleep(UPDATE_EVERY).await;
        }
    });
    // Ask the Bridge first, so the window's first glance is right; locating browsers is slower.
    let mut located = false;
    let mut checked = false;
    let mut start = launch.start_bridge;
    loop {
        match Bridge::existing(None).await {
            Ok(Some(bridge)) => {
                located = true;
                start = false;
                if !connected(&bridge, &link, &mut commands, &mut checked).await {
                    return;
                }
            }
            _ => {
                if std::mem::take(&mut start) && vexdom_path().is_some() {
                    link.send(Update::Starting);
                    link.done(cli(&["start"]).await, |_| "Vexdom is on.".into());
                    continue;
                }
                link.send(Update::Disconnected);
                if !located {
                    located = true;
                    let _ = link.browsers().await;
                }
                if !std::mem::replace(&mut checked, true) {
                    preflight(&link, None).await;
                }
                tokio::select! {
                    () = tokio::time::sleep(POLL) => {}
                    command = commands.recv() => match command {
                        Some(command) => link.offline(command).await,
                        None => return,
                    },
                }
            }
        }
    }
}

async fn status(bridge: &Bridge, link: &Link) -> bool {
    match bridge.status().await {
        Ok(status) => {
            link.send(Update::Status(status));
            true
        }
        Err(_) => false,
    }
}

async fn sessions(bridge: &Bridge, link: &Link) {
    let Ok(reply) = bridge.control.sessions_request().send().promise.await else {
        return;
    };
    let Ok(list) = reply.get().and_then(|reply| reply.get_sessions()) else {
        return;
    };
    let rows = list
        .iter()
        .map(|session| {
            json!({
                "id": text(session.get_id()),
                "browser": text(session.get_browser()),
                "label": text(session.get_label()),
                "lastSeenAt": text(session.get_last_seen_at()),
                "connected": session.get_connected(),
                "source": text(session.get_source()),
                "extensionVersion": text(session.get_extension_version()),
            })
        })
        .collect();
    link.send(Update::Sessions(rows));
}

fn text(reader: capnp::Result<capnp::text::Reader<'_>>) -> String {
    reader
        .ok()
        .and_then(|text| text.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

fn json_of(reader: capnp::Result<capnp::text::Reader<'_>>) -> Value {
    serde_json::from_str(&text(reader)).unwrap_or(Value::Null)
}

async fn pair_requests(bridge: &Bridge, link: &Link) {
    if let Ok(reply) = bridge.control.pair_requests_request().send().promise.await
        && let Ok(reply) = reply.get()
    {
        link.send(Update::PairRequests(json_of(reply.get_requests())));
    }
}

async fn take_notices(bridge: &Bridge, link: &Link) {
    if let Ok(reply) = bridge.control.take_notices_request().send().promise.await
        && let Ok(reply) = reply.get()
    {
        link.send(Update::Notices(json_of(reply.get_notices())));
    }
}

async fn confirmations(bridge: &Bridge, link: &Link) {
    link.send(Update::Confirmations(Value::Array(
        bridge.confirmations().await,
    )));
}

async fn agent_state(bridge: &Bridge) -> Option<Value> {
    let mut request = bridge.control.agent_request();
    request.get().set_set("");
    request.get().set_grant("");
    request.get().set_models("");
    let reply = request.send().promise.await.ok()?;
    Some(json_of(reply.get().ok()?.get_state()))
}

async fn agent(bridge: &Bridge, link: &Link) {
    if let Some(state) = agent_state(bridge).await {
        link.send(Update::Agent(state));
    }
}

/// `Control.preferences` or `Control.setPreference`'s answer, sent on as [`Update::Preferences`].
fn preferences_of(link: &Link, result: vexdom_core::result::ActionResult) {
    match result {
        Ok(now) => link.send(Update::Preferences(now)),
        Err(error) => link.send(Update::Error(error.message)),
    }
}

async fn preferences(bridge: &Bridge, link: &Link) {
    match bridge.control.preferences_request().send().promise.await {
        Ok(reply) => match reply.get() {
            Ok(reply) => preferences_of(link, vexdom_client::read_result(reply.get_result())),
            Err(error) => link.send(Update::Error(error.to_string())),
        },
        Err(error) => link.send(Update::Error(error.to_string())),
    }
}

async fn set_preference(bridge: &Bridge, link: &Link, change: &Value) {
    let mut call = bridge.control.set_preference_request();
    call.get().set_change(change.to_string());
    match call.send().promise.await {
        Ok(reply) => match reply.get() {
            Ok(reply) => preferences_of(link, vexdom_client::read_result(reply.get_result())),
            Err(error) => link.send(Update::Error(error.to_string())),
        },
        Err(error) => link.send(Update::Error(error.to_string())),
    }
}

async fn pair_code(bridge: &Bridge, link: &Link) {
    match bridge.control.pair_request().send().promise.await {
        Ok(reply) => match reply.get() {
            Ok(reply) => link.send(Update::PairCode {
                code: text(reply.get_code()),
                expires_at: reply.get_expires_at(),
            }),
            Err(error) => link.send(Update::Error(error.to_string())),
        },
        Err(error) => link.send(Update::Error(error.to_string())),
    }
}

async fn set_pairing(bridge: &Bridge, link: &Link, mode: &str) {
    let mut call = bridge.control.pairing_request();
    call.get().set_set(mode);
    match call.send().promise.await {
        Ok(reply) => {
            let error = reply
                .get()
                .map(|reply| text(reply.get_error()))
                .unwrap_or_default();
            if !error.is_empty() {
                link.send(Update::Error(error));
            }
        }
        Err(error) => link.send(Update::Error(error.to_string())),
    }
}

/// `revoke` with `session` (or every browser when empty).
async fn revoke(bridge: &Bridge, link: &Link, session: &str) {
    let mut call = bridge.control.revoke_request();
    call.get().set_session(session);
    call.get().set_origin("");
    match call.send().promise.await {
        Ok(reply) if session.is_empty() => {
            let revoked = reply.get().map_or(0, |reply| reply.get_revoked());
            link.send(Update::Said(match revoked {
                0 => "No browser was paired.".to_owned(),
                1 => "Unpaired 1 browser.".to_owned(),
                n => format!("Unpaired {n} browsers."),
            }));
        }
        Ok(_) => {}
        Err(error) => link.send(Update::Error(error.to_string())),
    }
}

/// Serves one connection until it drops; `false` when the window has gone.
async fn connected(
    bridge: &Bridge,
    link: &Link,
    commands: &mut UnboundedReceiver<Command>,
    checked: &mut bool,
) -> bool {
    let (changed, mut topics) = unbounded_channel::<Topic>();
    let mut watches = Vec::new();
    for (topic, approver, allowed) in [
        (Topic::PairRequestsChanged, Some("app"), false),
        (Topic::NoticesChanged, None, true),
        (Topic::SettingsChanged, None, false),
        (Topic::ConfirmationsChanged, None, false),
    ] {
        let changed = changed.clone();
        if let Ok(handle) = bridge
            .watch(topic, approver, allowed, move |topic, _| {
                let _ = changed.send(topic);
            })
            .await
        {
            watches.push(handle);
        }
    }
    if !status(bridge, link).await {
        return true;
    }
    sessions(bridge, link).await;
    pair_requests(bridge, link).await;
    take_notices(bridge, link).await;
    confirmations(bridge, link).await;
    agent(bridge, link).await;
    let _ = link.browsers().await;
    if !std::mem::replace(checked, true) {
        preflight(link, Some(bridge)).await;
    }

    let mut poll = tokio::time::interval(POLL);
    poll.reset();
    loop {
        tokio::select! {
            _ = poll.tick() => {
                if !status(bridge, link).await {
                    return true;
                }
                sessions(bridge, link).await;
            }
            topic = topics.recv() => match topic {
                Some(Topic::PairRequestsChanged) => {
                    pair_requests(bridge, link).await;
                    sessions(bridge, link).await;
                }
                Some(Topic::NoticesChanged) => take_notices(bridge, link).await,
                Some(Topic::SettingsChanged) => {
                    agent(bridge, link).await;
                    preferences(bridge, link).await;
                }
                Some(Topic::ConfirmationsChanged) => confirmations(bridge, link).await,
                // Not watched here.
                Some(Topic::ManifestChanged) | None => {}
            },
            command = commands.recv() => {
                let Some(command) = command else {
                    return false;
                };
                match command {
                    Command::Decide { request, allow, label } => {
                        let mut call = bridge.control.decide_request();
                        call.get().set_request(&request);
                        call.get().set_allow(allow);
                        call.get().set_label(&label);
                        match call.send().promise.await {
                            Ok(reply) => {
                                let error = reply
                                    .get()
                                    .map(|reply| text(reply.get_error()))
                                    .unwrap_or_default();
                                if !error.is_empty() {
                                    link.send(Update::Error(error));
                                }
                            }
                            Err(error) => link.send(Update::Error(error.to_string())),
                        }
                        pair_requests(bridge, link).await;
                        sessions(bridge, link).await;
                    }
                    Command::Revoke { session } => {
                        revoke(bridge, link, &session).await;
                        sessions(bridge, link).await;
                        let _ = link.browsers().await;
                    }
                    Command::RevokeAll => {
                        revoke(bridge, link, "").await;
                        sessions(bridge, link).await;
                        let _ = link.browsers().await;
                    }
                    Command::OpenNotice(id) => {
                        let mut call = bridge.control.open_notice_request();
                        call.get().set_notice(&id);
                        if let Err(error) = call.send().promise.await {
                            link.send(Update::Error(error.to_string()));
                        }
                    }
                    Command::Confirm { id, allow, remember } => {
                        if !bridge.confirm(&id, allow, remember).await {
                            link.send(Update::Error("That action stopped waiting for an answer.".into()));
                        }
                        confirmations(bridge, link).await;
                    }
                    Command::PairCode => pair_code(bridge, link).await,
                    Command::SetPairing(mode) => {
                        set_pairing(bridge, link, &mode).await;
                        status(bridge, link).await;
                    }
                    Command::Preferences => preferences(bridge, link).await,
                    Command::SetPreference(change) => set_preference(bridge, link, &change).await,
                    Command::Recheck => {
                        preflight(link, Some(bridge)).await;
                        agent(bridge, link).await;
                    }
                    Command::Refresh => {
                        sessions(bridge, link).await;
                        pair_requests(bridge, link).await;
                        confirmations(bridge, link).await;
                        agent(bridge, link).await;
                        let _ = link.browsers().await;
                    }
                    command @ (Command::Stop | Command::Restart | Command::Uninstall { .. }) => {
                        link.offline(command).await;
                        return true;
                    }
                    command @ (Command::Start
                    | Command::InstallFor { .. }
                    | Command::SetAgent { .. }
                    | Command::FixAgent) => {
                        link.offline(command).await;
                        agent(bridge, link).await;
                    }
                    command => link.offline(command).await,
                }
            }
        }
    }
}
