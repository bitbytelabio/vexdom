//! Vexdom Bridge (ADR-025): one tokio `current_thread` runtime inside a `LocalSet`. axum serves
//! `/health` and the WebSocket upgrades; each socket then runs Cap'n Proto RPC with the
//! `ExtensionGate` (extension role, by `Origin`) or `Control` (Bearer token) as its bootstrap.

pub mod agent;
mod artifacts;
mod auth_vault;
mod confirmations;
mod contact_sheet;
mod control;
mod devtools;
pub mod ensure;
mod flow;
mod gate;
mod http;
mod invoke;
pub mod link;
pub mod markdown;
pub mod native_host;
mod page;
pub mod quarantine;
pub mod react;
mod recording_video;
mod scheduler;
mod session;
mod webmcp;
pub mod wire;

pub use page::DELTA_ACTIONS;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::process::ExitCode;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::sync::Notify;
use vexdom_core::config::{self, read_agent_config};
use vexdom_core::guardrails::scope::anywhere;
use vexdom_core::guardrails::{
    Caller, Effect, GuardrailRequest, INJECT_ACTION, RUN_CODE_ACTION, decide, policy_from,
};
use vexdom_core::notices::NoticeDesk;
use vexdom_core::pairing::{DeskHost, PairingDesk};
use vexdom_core::preferences::{ConfigMark, mark_config, preferences_now};
use vexdom_core::proto::SOCKET_PROTOCOL_VERSION;
use vexdom_core::result::{ActionResult, failure};
use vexdom_core::{auth_store, downloads, lockfile, log, paths, skills, tools};
use vexdom_schema::{Topic, event_sink};

use agent::{AgentDeps, AgentSession, FileAnalyses};
use link::Link;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const IDLE_EXIT: Duration = Duration::from_secs(30 * 60);
const BINDING_IDLE_MS: i64 = 2 * 60 * 1000;
const LINK_SILENCE: Duration = Duration::from_secs(60);
const LIVENESS_CHECK: Duration = Duration::from_secs(20);
/// A browser installed since, or a registration an older Bridge wrote, is put right this often.
const REREGISTER: Duration = Duration::from_secs(60 * 60);

pub struct Options {
    /// `--no-idle-exit`: stay up with nothing connected.
    pub no_idle_exit: bool,
}

/// `vexdom daemon`: runs until stopped, idle, or signalled.
pub fn run(opts: Options) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("vexdom-mcpd: {error}");
            return ExitCode::FAILURE;
        }
    };
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        match Daemon::start(!opts.no_idle_exit).await {
            Ok(daemon) => {
                daemon.wait().await;
                ExitCode::SUCCESS
            }
            Err(error) => {
                log::log_detail("daemon failed to start", &error);
                eprintln!("vexdom-mcpd: {error}");
                ExitCode::FAILURE
            }
        }
    })
}

/// Which browser a control connection's calls reach while it keeps calling.
#[derive(Debug, Default, Clone)]
pub(crate) struct Binding {
    pub id: Option<String>,
    pub used_at: i64,
}

/// A control connection's event subscription.
pub(crate) struct Watcher {
    pub id: u64,
    pub conn: u64,
    pub topic: Topic,
    pub approver: Option<String>,
    pub sink: event_sink::Client,
}

/// `Control.describe`'s answer.
pub(crate) struct Described {
    /// Changes whenever the offer may have: never 0.
    pub stamp: u64,
    /// `None` when the caller already holds the offer with this stamp.
    pub tools: Option<Vec<Value>>,
    pub reserved: Vec<String>,
    /// Names of the tools the run is not offered.
    pub withheld: Vec<String>,
}

/// A non-zero `describe` stamp for `parts`.
fn stamp_of(parts: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish().max(1)
}

pub(crate) struct Inner {
    pub token: String,
    pub port: Cell<u16>,
    idle_exit: bool,
    pub links: RefCell<Vec<Rc<Link>>>,
    sessions: RefCell<Vec<(u64, Rc<AgentSession>)>>,
    pub analyses: Rc<FileAnalyses>,
    pub controls: Cell<usize>,
    serial: Cell<u64>,
    /// When this daemon started, in ns: in every `describe` stamp, so a restart never matches.
    boot: u64,
    activity: Cell<u64>,
    pub desk: RefCell<PairingDesk<DeskBridge>>,
    pub notices: RefCell<NoticeDesk>,
    pub watchers: RefCell<Vec<Watcher>>,
    config_seen: RefCell<ConfigMark>,
    offered: RefCell<Rc<Vec<Value>>>,
    pub bundled: Rc<Vec<Value>>,
    firefox: Rc<Vec<Value>>,
    pub scheduler: scheduler::Scheduler,
    changed: Notify,
    stopping: Notify,
    stopped: Cell<bool>,
    connected: Arc<AtomicBool>,
    waiters: RefCell<HashMap<String, Rc<Notify>>>,
    pages: RefCell<HashMap<u64, page::PageState>>,
    pub video: RefCell<recording_video::Video>,
    confirmations: RefCell<confirmations::Confirmations>,
    pub flows: RefCell<flow::Recorders>,
}

/// The pairing desk's view of the daemon.
pub(crate) struct DeskBridge(Weak<Inner>);

impl DeskHost for DeskBridge {
    fn port(&self) -> u16 {
        self.0.upgrade().map_or(0, |inner| inner.port.get())
    }

    fn mode(&self) -> String {
        config::pairing_mode(&read_agent_config())
    }

    fn approver(&self) -> String {
        self.0.upgrade().map_or_else(
            || "terminal".to_owned(),
            |inner| Daemon(inner).approver_now(),
        )
    }

    fn changed(&self) {
        if let Some(inner) = self.0.upgrade() {
            Daemon(inner).notify(Topic::PairRequestsChanged, &Value::Null);
        }
    }

    fn wake(&self, install_id: &str) {
        if let Some(inner) = self.0.upgrade()
            && let Some(waiter) = inner.waiters.borrow_mut().remove(install_id)
        {
            waiter.notify_waiters();
        }
    }
}

#[derive(Clone)]
pub struct Daemon(pub(crate) Rc<Inner>);

impl Daemon {
    /// Binds, writes the lockfile, and starts serving on the current `LocalSet`.
    pub async fn start(idle_exit: bool) -> Result<Self, String> {
        let swept = downloads::sweep_downloads();
        if swept > 0 {
            log::log(&format!(
                "swept {swept} expired download{}",
                if swept == 1 { "" } else { "s" }
            ));
        }
        let ports = paths::daemon_ports()?;
        let listener = http::bind_first(&ports)
            .await
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let token = vexdom_core::rand::base64url(24);
        let bundled = Rc::new(tools::chromium().tools.clone());
        let firefox = Rc::new(tools::firefox().tools.clone());
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| Inner {
            token: token.clone(),
            port: Cell::new(port),
            idle_exit,
            links: RefCell::new(Vec::new()),
            sessions: RefCell::new(Vec::new()),
            analyses: Rc::new(FileAnalyses::default()),
            controls: Cell::new(0),
            serial: Cell::new(0),
            boot: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(1, |since| since.as_nanos() as u64),
            activity: Cell::new(0),
            desk: RefCell::new(PairingDesk::new(DeskBridge(weak.clone()))),
            notices: RefCell::new(NoticeDesk::default()),
            watchers: RefCell::new(Vec::new()),
            config_seen: RefCell::new(mark_config()),
            offered: RefCell::new(Rc::clone(&bundled)),
            bundled,
            firefox,
            scheduler: scheduler::Scheduler::new(weak.clone()),
            changed: Notify::new(),
            stopping: Notify::new(),
            stopped: Cell::new(false),
            connected: Arc::new(AtomicBool::new(false)),
            waiters: RefCell::new(HashMap::new()),
            pages: RefCell::new(HashMap::new()),
            video: RefCell::new(recording_video::Video::default()),
            confirmations: RefCell::default(),
            flows: RefCell::default(),
        });
        let daemon = Daemon(inner);
        let pid = std::process::id();
        let lock = lockfile::Lockfile {
            pid,
            port,
            token,
            protocol_version: SOCKET_PROTOCOL_VERSION,
            daemon_version: VERSION.to_owned(),
        };
        lockfile::write(&paths::lockfile(), &lock)
            .map_err(|error| format!("cannot write {}: {error}", paths::lockfile().display()))?;
        lockfile::release_wake();
        http::serve(daemon.clone(), listener);
        daemon.spawn_housekeeping();
        daemon.0.scheduler.start();
        log::log(&format!(
            "daemon {VERSION} listening on 127.0.0.1:{port} (pid {pid})"
        ));
        Ok(daemon)
    }

    /// Resolves once the daemon has stopped.
    pub async fn wait(&self) {
        while !self.0.stopped.get() {
            self.0.stopping.notified().await;
        }
    }

    pub fn port(&self) -> u16 {
        self.0.port.get()
    }

    pub fn token(&self) -> &str {
        &self.0.token
    }

    fn spawn_housekeeping(&self) {
        let daemon = self.clone();
        tokio::task::spawn_local(async move { daemon.idle_loop().await });
        let daemon = self.clone();
        tokio::task::spawn_local(async move { daemon.signals().await });
        tokio::task::spawn_local(async move {
            loop {
                reregister_native_host();
                tokio::time::sleep(REREGISTER).await;
            }
        });
        #[cfg(target_os = "macos")]
        tokio::task::spawn_blocking(|| {
            let dropped = quarantine::drop_quarantined_bun_libraries(
                &std::env::temp_dir(),
                nix::unistd::getuid().as_raw(),
            );
            for file in dropped {
                log::log(&format!(
                    "removed {}, a library an agent unpacked while the Bridge ran quarantined",
                    file.display()
                ));
            }
        });
        let daemon = self.clone();
        tokio::task::spawn_local(async move {
            loop {
                tokio::time::sleep(LIVENESS_CHECK).await;
                for link in daemon.open_links() {
                    if link.last_ping.get().elapsed() > LINK_SILENCE {
                        log::log(&format!("{} went silent; dropping it", link.label()));
                        link.close("no keepalive");
                    }
                }
            }
        });
    }

    async fn idle_loop(&self) {
        if !self.0.idle_exit {
            return;
        }
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.busy() {
                changed.await;
                continue;
            }
            tokio::select! {
                () = tokio::time::sleep(IDLE_EXIT) => {
                    if !self.busy() {
                        log::log("idle with no clients; exiting");
                        self.stop();
                        return;
                    }
                }
                () = &mut changed => {}
            }
        }
    }

    async fn signals(&self) {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut interrupt)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = interrupt.recv() => {}
        }
        self.stop();
    }

    fn busy(&self) -> bool {
        !self.open_links().is_empty() || self.0.controls.get() > 0
    }

    /// Something that idle exit counts changed.
    pub(crate) fn activity_changed(&self) {
        self.0
            .connected
            .store(!self.open_links().is_empty(), Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }

    pub fn stop(&self) {
        if self.0.stopped.replace(true) {
            return;
        }
        let links = self.0.links.borrow().clone();
        for link in links {
            link.close("daemon shutting down");
        }
        lockfile::remove_if_ours(&paths::lockfile(), std::process::id());
        log::log("daemon stopped");
        self.0.stopping.notify_waiters();
    }

    pub(crate) fn next_serial(&self) -> u64 {
        self.0.serial.set(self.0.serial.get() + 1);
        self.0.serial.get()
    }

    /// A counter rather than a clock: the later of two touches has to win.
    pub(crate) fn touch(&self) -> u64 {
        self.0.activity.set(self.0.activity.get() + 1);
        self.0.activity.get()
    }

    pub(crate) fn open_links(&self) -> Vec<Rc<Link>> {
        self.0
            .links
            .borrow()
            .iter()
            .filter(|link| link.is_open())
            .cloned()
            .collect()
    }

    pub(crate) fn link(&self, install_id: &str) -> Option<Rc<Link>> {
        self.0
            .links
            .borrow()
            .iter()
            .find(|link| link.install_id == install_id)
            .cloned()
    }

    /// The browser the user was last in.
    pub(crate) fn active_link(&self) -> Option<Rc<Link>> {
        self.open_links()
            .into_iter()
            .max_by_key(|link| link.last_active.get())
    }

    /// A burst of calls stays with one browser; a quiet caller follows the user.
    pub(crate) fn route_for(&self, binding: &RefCell<Binding>) -> Option<Rc<Link>> {
        let now = log::now_ms();
        let mut binding = binding.borrow_mut();
        let held = binding.id.as_deref().and_then(|id| self.link(id));
        let target = match held {
            Some(link) if link.is_open() && now - binding.used_at < BINDING_IDLE_MS => Some(link),
            _ => self.active_link(),
        };
        binding.id = target.as_ref().map(|link| link.install_id.clone());
        binding.used_at = now;
        target
    }

    pub(crate) fn in_sync(&self, link: &Link) -> bool {
        tools::bundled(&link.manifest_hash).is_some()
    }

    pub(crate) fn offered_tools(&self) -> Rc<Vec<Value>> {
        self.active_link().map_or_else(
            || Rc::clone(&self.0.bundled),
            |link| Rc::clone(&link.tools.borrow()),
        )
    }

    /// A worker restart closes one link and opens another in the same tick, so the comparison
    /// waits for the tick to end.
    pub(crate) fn settle_offer(&self) {
        let daemon = self.clone();
        tokio::task::spawn_local(async move {
            tokio::task::yield_now().await;
            let now = daemon.offered_tools();
            if Rc::ptr_eq(&now, &daemon.0.offered.borrow()) {
                return;
            }
            *daemon.0.offered.borrow_mut() = now;
            daemon.notify(Topic::ManifestChanged, &Value::Null);
        });
    }

    pub(crate) fn approver_now(&self) -> String {
        let app = self.0.watchers.borrow().iter().any(|watcher| {
            watcher.topic == Topic::PairRequestsChanged
                && watcher.approver.as_deref() == Some("app")
        });
        if app { "app".into() } else { "terminal".into() }
    }

    /// Sends an event to every watcher of `topic` (notices only to watchers that may show them).
    pub(crate) fn notify(&self, topic: Topic, payload: &Value) {
        let showers = self.0.notices.borrow().showers();
        let sinks: Vec<event_sink::Client> = self
            .0
            .watchers
            .borrow()
            .iter()
            .filter(|watcher| watcher.topic == topic)
            .filter(|watcher| topic != Topic::NoticesChanged || showers.contains(&watcher.id))
            .map(|watcher| watcher.sink.clone())
            .collect();
        for sink in sinks {
            let mut request = sink.event_request();
            request.get().set_topic(topic);
            request.get().set_payload(wire::json_text(payload));
            tokio::task::spawn_local(async move {
                let _ = request.send().promise.await;
            });
        }
    }

    pub(crate) fn add_watcher(&self, watcher: Watcher, allowed: bool) {
        if watcher.topic == Topic::NoticesChanged {
            self.0.notices.borrow_mut().watch(watcher.id, allowed);
        }
        self.0.watchers.borrow_mut().push(watcher);
    }

    pub(crate) fn remove_watcher(&self, id: u64) {
        self.0
            .watchers
            .borrow_mut()
            .retain(|watcher| watcher.id != id);
        self.0.notices.borrow_mut().unwatch(id);
    }

    /// Waits for a pairing request about `install_id` to be decided, or `ms`.
    pub(crate) async fn settled(&self, install_id: &str, ms: u64) {
        let waiter = Rc::clone(
            self.0
                .waiters
                .borrow_mut()
                .entry(install_id.to_owned())
                .or_insert_with(|| Rc::new(Notify::new())),
        );
        let _ = tokio::time::timeout(Duration::from_millis(ms), waiter.notified()).await;
    }

    pub(crate) fn skill_catalog_now(&self, refresh: bool) -> ActionResult {
        let config = read_agent_config();
        let skills: Vec<Value> = skills::load_skills()
            .into_iter()
            .map(|skill| {
                json!({"name": skill.name, "description": skill.description, "category": skill.category,
                       "domains": skill.domains, "source": skill.source})
            })
            .collect();
        Ok(
            json!({"agent": config.agent, "skills": skills, "agentSkills": agent::agent_skills(&config, refresh)}),
        )
    }

    pub(crate) fn push_skill_catalog(&self, link: &Link) {
        link.skill_catalog(&self.skill_catalog_now(false));
    }

    pub(crate) fn push_agent_state(&self, link: Rc<Link>) {
        tokio::task::spawn_local(async move {
            let state = agent::agent_state(&read_agent_config(), false).await;
            link.agent_info(&Ok(state));
        });
    }

    pub(crate) fn share_skill_catalog(&self) {
        for link in self.open_links() {
            self.push_skill_catalog(&link);
        }
    }

    pub(crate) fn announce_agent(&self, except: Option<u64>) {
        for link in self.open_links() {
            if Some(link.serial) == except {
                continue;
            }
            self.push_skill_catalog(&link);
            self.push_agent_state(link);
        }
    }

    pub(crate) fn reset_conversations(&self) {
        let sessions: Vec<Rc<AgentSession>> = self
            .0
            .sessions
            .borrow()
            .iter()
            .map(|(_, session)| Rc::clone(session))
            .collect();
        for session in sessions {
            session.handle(agent::AgentRequest::Reset { session_id: None });
        }
    }

    /// Hands every open surface what config.json now says, once per change.
    pub(crate) fn config_changed(&self, except: Option<u64>) {
        let mark = mark_config();
        let agent_moved = {
            let mut seen = self.0.config_seen.borrow_mut();
            if mark.file == seen.file {
                return;
            }
            let moved = mark.agent != seen.agent;
            *seen = mark;
            moved
        };
        let preferences = Ok(preferences_now());
        for link in self.open_links() {
            if Some(link.serial) == except {
                continue;
            }
            link.preferences_info(&preferences);
            if agent_moved {
                self.push_skill_catalog(&link);
                self.push_agent_state(link);
            }
        }
        self.notify(Topic::SettingsChanged, &Value::Null);
    }

    pub(crate) fn session(&self, link: &Rc<Link>) -> Rc<AgentSession> {
        let held = self
            .0
            .sessions
            .borrow()
            .iter()
            .find(|(serial, _)| *serial == link.serial)
            .map(|(_, session)| Rc::clone(session));
        if let Some(session) = held {
            return session;
        }
        let weak_link = Rc::downgrade(link);
        let daemon = self.clone();
        let invoke_link = weak_link.clone();
        let emit_link = weak_link.clone();
        let draft_link = weak_link;
        let counter = self.clone();
        let deps = AgentDeps {
            invoke: Rc::new(move |action, input, opts| {
                let daemon = daemon.clone();
                let link = invoke_link.upgrade();
                Box::pin(async move {
                    // A run's refs (`e5`) name elements of the snapshots it took, as a control
                    // client's do.
                    let mut input = input;
                    if let Some(open) = link.as_ref().filter(|link| link.is_open())
                        && let Err(error) = daemon.rewrite_refs(open, &mut input)
                    {
                        return Err(error).into();
                    }
                    daemon.invoke_on(link, &action, input, &opts).await
                })
            }),
            emit: Rc::new(move |id, event| {
                if let Some(link) = emit_link.upgrade() {
                    link.run(id, &event);
                }
            }),
            draft: Rc::new(move |id, draft| {
                if let Some(link) = draft_link.upgrade() {
                    link.site_map_draft(id, draft);
                }
            }),
            running: Rc::new(move || {
                counter
                    .0
                    .sessions
                    .borrow()
                    .iter()
                    .map(|(_, session)| session.running())
                    .sum()
            }),
            analyses: Rc::clone(&self.0.analyses),
        };
        let session = AgentSession::new(deps);
        self.0
            .sessions
            .borrow_mut()
            .push((link.serial, Rc::clone(&session)));
        session
    }

    pub(crate) fn session_running(&self, run_id: &str) -> Option<(u64, Rc<AgentSession>)> {
        self.0
            .sessions
            .borrow()
            .iter()
            .find(|(_, session)| session.owns(run_id))
            .map(|(serial, session)| (*serial, Rc::clone(session)))
    }

    pub(crate) fn link_by_serial(&self, serial: u64) -> Option<Rc<Link>> {
        self.0
            .links
            .borrow()
            .iter()
            .find(|link| link.serial == serial)
            .cloned()
    }

    /// `describeFor`: a run's own offer, or what the browser a call would reach offers.
    pub(crate) fn describe_for(
        &self,
        run_id: Option<&str>,
        binding: &RefCell<Binding>,
        known: u64,
    ) -> Described {
        fn name(tool: &Value) -> &str {
            tool.get("name").and_then(Value::as_str).unwrap_or("")
        }
        let withheld_by = |withheld: &[String], tool: &Value| {
            let name = name(tool);
            withheld.iter().any(|kept| kept == name)
        };
        if let Some(run_id) = run_id
            && let Some((serial, session)) = self.session_running(run_id)
            && let Some(offer) = session.offer_for(run_id)
            && let Some(link) = self.link_by_serial(serial)
        {
            let stamp = stamp_of((
                self.0.boot,
                1u8,
                run_id,
                link.serial,
                link.tools_serial.get(),
                &offer.withheld,
                &offer.reserved,
            ));
            let link_tools = Rc::clone(&link.tools.borrow());
            let all = || link_tools.iter().chain(tools::bridge().tools.iter());
            let withheld = all()
                .filter(|tool| withheld_by(&offer.withheld, tool))
                .map(|tool| name(tool).to_owned())
                .collect();
            let tools = (known != stamp).then(|| {
                all()
                    .filter(|tool| !withheld_by(&offer.withheld, tool))
                    .cloned()
                    .collect()
            });
            return Described {
                stamp,
                tools,
                reserved: offer.reserved,
                withheld,
            };
        }
        let route = self.route_for(binding);
        let config = read_agent_config();
        let policy = policy_from(&config.guardrails, &config.require_approval);
        let scope = anywhere();
        let denied = |action: &str| {
            decide(
                &GuardrailRequest {
                    action,
                    input: &json!({}),
                    caller: Caller::External,
                    scope: &scope,
                },
                &policy,
            )
            .effect
                == Effect::Deny
        };
        let deny_inject = denied(INJECT_ACTION);
        let deny_run = denied(RUN_CODE_ACTION);
        let stamp = stamp_of((
            self.0.boot,
            2u8,
            route.as_ref().map_or(0, |link| link.serial),
            route.as_ref().map_or(0, |link| link.tools_serial.get()),
            deny_inject,
            deny_run,
        ));
        let tools = (known != stamp).then(|| {
            let tools = route.as_ref().map_or_else(
                || Rc::clone(&self.0.bundled),
                |link| Rc::clone(&link.tools.borrow()),
            );
            tools
                .iter()
                .chain(tools::bridge().tools.iter())
                .filter(|tool| {
                    let name = name(tool);
                    !(name == INJECT_ACTION && deny_inject)
                        && !(name == RUN_CODE_ACTION && deny_run)
                        && !tools::DAEMON_INTERNAL_ACTIONS.contains(&name)
                })
                .cloned()
                .collect()
        });
        Described {
            stamp,
            tools,
            reserved: Vec::new(),
            withheld: Vec::new(),
        }
    }

    /// Lists a link that proved itself: supersedes the same profile's older link.
    pub(crate) fn accept_link(&self, link: Rc<Link>) {
        if let Some(old) = self.link(&link.install_id) {
            old.close("superseded by a newer connection");
        }
        let known = tools::bundled(&link.manifest_hash);
        *link.tools.borrow_mut() = match known {
            Some(manifest) if manifest.hash == tools::firefox().hash => Rc::clone(&self.0.firefox),
            _ => Rc::clone(&self.0.bundled),
        };
        link.tools_serial.set(self.next_serial());
        let daemon = self.clone();
        link.set_on_close(move |closing| {
            let serial = closing.serial;
            let position = daemon
                .0
                .sessions
                .borrow()
                .iter()
                .position(|(s, _)| *s == serial);
            if let Some(at) = position {
                let (_, session) = daemon.0.sessions.borrow_mut().remove(at);
                session.dispose();
            }
            daemon.0.scheduler.link_closed(&closing.install_id);
            daemon.0.analyses.cancel_owned_by(serial);
            daemon
                .0
                .links
                .borrow_mut()
                .retain(|kept| kept.serial != serial);
            daemon.forget_page(serial);
            log::log(&format!("{} disconnected", closing.label()));
            daemon.activity_changed();
            daemon.settle_offer();
        });
        self.0.links.borrow_mut().push(Rc::clone(&link));
        self.settle_offer();
        log::log(&format!(
            "extension {} connected from {} (manifest {})",
            link.extension_version,
            link.label(),
            if known.is_some() {
                "in sync"
            } else {
                "DRIFTED"
            }
        ));
        self.activity_changed();
        self.push_agent_state(Rc::clone(&link));
        self.push_skill_catalog(&link);
        link.preferences_info(&Ok(preferences_now()));
        link.task_list(&Ok(self.0.scheduler.list()));
        self.0.scheduler.link_opened();
        if known.is_none() {
            let daemon = self.clone();
            tokio::task::spawn_local(async move {
                match link.describe().await.filter(|tools| !tools.is_empty()) {
                    Some(reported) => {
                        log::log(&format!(
                            "adopted {} tools from {} (bundled list was {})",
                            reported.len(),
                            link.label(),
                            daemon.0.bundled.len()
                        ));
                        *link.tools.borrow_mut() = Rc::new(reported);
                        link.tools_serial.set(daemon.next_serial());
                        daemon.settle_offer();
                    }
                    None => log::log(&format!(
                        "{} drifted but its manifest could not be fetched; serving it the bundled tool list",
                        link.label()
                    )),
                }
            });
        }
    }

    pub(crate) fn session_summaries(&self) -> Vec<Value> {
        auth_store::list_sessions()
            .into_iter()
            .map(|session| {
                let id = session.id().to_owned();
                let connected = self.link(&id).is_some_and(|link| link.is_open());
                let mut summary = serde_json::Map::new();
                summary.insert("id".into(), json!(id));
                if let Some(browser) = &session.browser {
                    summary.insert("browser".into(), json!(browser));
                }
                summary.insert("origin".into(), json!(session.origin));
                if let Some(label) = &session.label {
                    summary.insert("label".into(), json!(label));
                }
                if let Some(by) = &session.paired_by {
                    summary.insert("pairedBy".into(), json!(by));
                }
                summary.insert("extensionVersion".into(), json!(session.extension_version));
                summary.insert("pairedAt".into(), json!(session.paired_at));
                summary.insert("lastSeenAt".into(), json!(session.last_seen_at));
                summary.insert(
                    "source".into(),
                    json!(vexdom_core::pairing::source_of(&session.origin)),
                );
                summary.insert("connected".into(), json!(connected));
                Value::Object(summary)
            })
            .collect()
    }

    pub(crate) fn health_connected(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0.connected)
    }
}

/// Unknown action (reserved prefix) and offline link answers, shared by every path.
pub(crate) fn unknown_action(action: &str) -> ActionResult {
    failure("UNKNOWN_ACTION", format!("Unknown action \"{action}\"."))
}

/// Re-registers the native host off the event loop: it reads browser folders and may run `plutil`.
pub(crate) fn reregister_native_host() {
    if let Ok(exe) = std::env::current_exe() {
        tokio::task::spawn_blocking(move || native_host::refresh_native_host(&exe));
    }
}
