//! The window: Overview (the power switch and its lamp, browsers asking to pair, actions waiting
//! for approval, notices), Browsers (pairing, adding the extension, paired browsers), Agent
//! (Claude Code and its model), Settings (General, Skills, Activity, Logs) and About (versions,
//! help, the app's own update). While a required first-run check fails, the first-run screen
//! stands in for the tabs. The look is the appliance face in `theme.rs`; the direction contract
//! is `.impeccable/surfaces/desktop-src-ui-rs.md`.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, Div, ElementId, Entity, Focusable, FontWeight,
    Hsla, ScrollHandle, SharedString, Stateful, Window, div, prelude::*, px,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::bridge::{Command, CommandLine, RELEASES, Update, VERSION};
use crate::log_tail::{LogTail, at_bottom};
use crate::login_item::{self, LoginState};
use crate::model::{
    AGENT_MISSING, AgentCard, ApprovalHost, BrowserRow, Confirmation, DownloadRow, EFFECTS,
    Headline, Lamp, NoticeBook, PAIRING_MODES, PairRequest, Preferences, SKILL_SOURCES, SessionRow,
    SkillRow, Unpacked, UpdateState, agent_card, approval_hosts, browser_rows, bug_report_url,
    confirmations, countdown, download_rows, facts, filter_skills, headline, mcp_command, notices,
    pair_requests, pairing_code, pairing_line, preferences, reload_needed, sessions, short_version,
    skill_rows, title_case, unpacked,
};
use crate::preflight::{Check, Fix, Outcome, Preflight};
use crate::settings::{self, Appearance, Settings};
use crate::text_input::{FieldEvent, TextInput};
use crate::theme::{self, Palette, Tone};
use crate::tray::{self, Pick, Tray};

/// Room for the traffic lights at the left of the title strip.
const TITLE_STRIP: f32 = 40.;

/// The Bridge keeps at most this many characters of a browser's name.
const LABEL_LIMIT: usize = 40;

/// The most characters the skills filter takes.
const FILTER_LIMIT: usize = 60;

/// How long the status strip keeps a message.
const LINE_LIFE: Duration = Duration::from_secs(8);

/// How long it keeps the steps for loading the unpacked extension: they are followed elsewhere.
const STEPS_LIFE: Duration = Duration::from_secs(45);

/// How long a control stays busy if no answer comes back.
const PENDING_LIMIT: Duration = Duration::from_secs(20);

/// A restart stops one Bridge and waits for the next.
const RESTART_LIMIT: Duration = Duration::from_secs(60);

/// How often Settings → Logs reads the log while it shows.
const LOG_POLL: Duration = Duration::from_secs(1);

const LOG_HEIGHT: f32 = 360.;

const GET_CHROME: &str = "https://www.google.com/chrome/";
const DOCS: &str = "https://github.com/bitbytelabio/vexdom/tree/main/docs";

/// What reaches the window: the bridge thread's updates and the menu-bar item's clicks.
pub enum Inbound {
    Bridge(Update),
    Menu(Pick),
}

/// A command sent and not answered yet; its control is dimmed and ignores clicks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Start,
    Stop,
    Restart,
    Decide,
    Confirm,
}

impl Pending {
    fn limit(self) -> Duration {
        match self {
            Self::Restart => RESTART_LIMIT,
            _ => PENDING_LIMIT,
        }
    }
}

/// The tabs, in the order ⌘1…⌘5 pick them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Browsers,
    Agent,
    Settings,
    About,
}

impl Tab {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Overview, "Overview"),
        (Self::Browsers, "Browsers"),
        (Self::Agent, "Agent"),
        (Self::Settings, "Settings"),
        (Self::About, "About"),
    ];
}

/// Settings' own pages, in its side list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Skills,
    Activity,
    Logs,
}

impl Page {
    const ALL: [(Self, &'static str); 4] = [
        (Self::General, "General"),
        (Self::Skills, "Skills"),
        (Self::Activity, "Activity"),
        (Self::Logs, "Logs"),
    ];
}

/// The last thing a command said, or went wrong with.
struct Line {
    text: String,
    error: bool,
}

pub struct AppView {
    commands: UnboundedSender<Command>,
    tray: Option<Tray>,
    tab: Tab,
    page: Page,
    /// Whether the bridge thread has answered once; until then the window claims nothing.
    known: bool,
    /// `Control.status`, or `None` while no Bridge answers.
    status: Option<Value>,
    sessions: Vec<SessionRow>,
    requests: Vec<PairRequest>,
    /// Requests and confirmations already announced (notification, menu-bar line).
    announced: HashSet<String>,
    /// The name for the first request, and which request it was filled in for.
    label: Entity<TextInput>,
    label_for: Option<String>,
    /// Focus the name field on the next frame (a new request arrived).
    focus_label: bool,
    agent: Option<AgentCard>,
    browsers: Vec<BrowserRow>,
    unpacked: Unpacked,
    notices: NoticeBook,
    confirmations: Vec<Confirmation>,
    /// A pairing code, shown as `XXXX-YYYY`, and when it runs out (ms since the epoch).
    code: Option<(String, i64)>,
    /// Bumped with every code, so an old countdown stops.
    code_seq: u64,
    /// "Unpair all" asked to be confirmed.
    unpair_all: bool,
    line: Option<Line>,
    /// Bumped with every message, so an old timer never clears a newer one.
    line_seq: u64,
    pending: Option<Pending>,
    pending_seq: u64,
    settings: Settings,
    /// The first-run checks; `None` until their first answer.
    preflight: Option<Preflight>,
    /// The first-run screen stands in for the tabs.
    first_run: bool,
    /// Opened from About: it stays until Continue, even with every check passing.
    first_run_asked: bool,
    /// A first-run fix runs.
    fixing: bool,
    npm: bool,
    preferences: Option<Preferences>,
    skills: Vec<SkillRow>,
    skill_filter: Entity<TextInput>,
    /// The source chip picked; `None` shows every source.
    skill_source: Option<&'static str>,
    approvals: Vec<ApprovalHost>,
    downloads: Vec<DownloadRow>,
    log: LogTail,
    log_scroll: ScrollHandle,
    /// Settings → Logs' poll runs.
    log_polling: bool,
    command_line: CommandLine,
    /// "Open at login"; `None` when the app runs unbundled.
    login: Option<LoginState>,
    /// "Uninstall…" asked to be confirmed.
    uninstall: bool,
    keep_skills: bool,
    update: UpdateState,
    macos: String,
}

/// Posts an OS notification from the executor's background threads: on the main thread, macOS's
/// delivery spins a nested run loop for up to two seconds inside a GPUI update.
fn notify(cx: &mut Context<AppView>, title: String, body: String) {
    cx.background_executor()
        .spawn(async move {
            let _ = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .show();
        })
        .detach();
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

fn lamp_color(p: &Palette, lamp: Lamp) -> Hsla {
    match lamp {
        Lamp::Off => p.lamp_off,
        Lamp::Waiting => p.lamp_waiting,
        Lamp::Connected => p.lamp_on,
    }
}

/// The indicator lamp: a small lit disc with a lighter rim, no glow.
fn lamp(size: f32, color: Hsla) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(color)
        .border_1()
        .border_color(gpui::white().opacity(0.35))
}

fn heading(p: &Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.ink)
        .child(text.into())
}

fn quiet(p: &Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .text_color(p.label)
        .child(text.into())
}

/// One row of a list inside a face: what it is on the left, its action on the right.
fn row(title: impl Into<SharedString>, detail: impl IntoElement, action: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .py(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.into()),
                )
                .child(detail),
        )
        .child(div().flex().flex_none().child(action))
}

/// Rows separated by hairlines in the well colour.
fn rows(p: &Palette, children: Vec<Div>) -> Div {
    let mut list = div().flex().flex_col();
    for (index, child) in children.into_iter().enumerate() {
        list = list.child(if index == 0 {
            child
        } else {
            child.border_t_1().border_color(p.well)
        });
    }
    list
}

/// A face holding a titled list, or the sentence for an empty one.
fn list_face(
    p: &Palette,
    title: impl Into<SharedString>,
    entries: Vec<Div>,
    empty: impl Into<SharedString>,
) -> Div {
    p.panel()
        .px(px(20.))
        .py(px(8.))
        .child(heading(p, title).py(px(6.)))
        .child(if entries.is_empty() {
            quiet(p, empty).py(px(10.))
        } else {
            rows(p, entries)
        })
}

/// The recessed track a set of chips sits in, like the tabs.
fn track(p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(7.))
        .bg(p.well_hover)
}

/// One choice in a track; the chosen one is a raised face, like the selected tab.
fn chip(
    p: &Palette,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    chosen: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id.into())
        .px(px(12.))
        .py(px(3.))
        .rounded(px(5.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(label.into());
    if chosen {
        base.bg(p.face).text_color(p.ink).shadow(p.lift())
    } else {
        let ink = p.ink;
        base.text_color(p.label)
            .hover(move |style| style.text_color(ink))
    }
}

/// An on/off switch: ink with the knob right while on, recessed while off.
fn switch(p: &Palette, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .w(px(38.))
        .h(px(22.))
        .flex_none()
        .flex()
        .items_center()
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { p.ink } else { p.well_hover })
        .when(on, |switch| switch.justify_end())
        .child(div().size(px(18.)).rounded_full().bg(p.face))
}

/// A box that is ticked or not, with its label.
fn checkbox(p: &Palette, id: impl Into<ElementId>, on: bool, label: &'static str) -> Stateful<Div> {
    let tick = div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if on { p.ink } else { p.label })
        .text_size(px(11.))
        .text_color(p.face)
        .when(on, |tick| tick.bg(p.ink).child("✓"));
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .text_size(px(13.))
        .child(tick)
        .child(label)
}

/// A small tag beside a row's detail, with an amber lamp: something to do in the browser.
fn badge(p: &Palette, text: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(7.))
        .py(px(1.))
        .rounded(px(5.))
        .bg(p.well)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.ink)
        .child(lamp(6., p.lamp_waiting))
        .child(text)
}

/// A lamp and its line, as a row's detail.
fn lamp_line(p: &Palette, state: Lamp, text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(lamp(8., lamp_color(p, state)))
        .child(
            div()
                .text_size(px(13.))
                .text_color(color)
                .child(text.into()),
        )
}

impl AppView {
    /// `inbox` carries the bridge thread's updates; `menu` is where the menu-bar item's clicks go
    /// (the same channel).
    pub fn new(
        commands: UnboundedSender<Command>,
        mut inbox: UnboundedReceiver<Inbound>,
        menu: UnboundedSender<Inbound>,
        settings: Settings,
        cx: &mut Context<Self>,
    ) -> Self {
        let tray = Tray::try_new(move |pick| {
            let _ = menu.send(Inbound::Menu(pick));
        })
        .inspect_err(|error| eprintln!("vexdom-desktop: no menu-bar item ({error})"))
        .ok();
        cx.spawn(async move |this, cx| {
            while let Some(message) = inbox.recv().await {
                if this
                    .update(cx, |view, cx| view.receive(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let label = cx.new(|cx| TextInput::new("Name this browser", LABEL_LIMIT, cx));
        cx.subscribe(&label, |view, _, event: &FieldEvent, cx| match event {
            FieldEvent::Submitted => view.decide_first(true, cx),
            FieldEvent::Cancelled => view.decide_first(false, cx),
        })
        .detach();
        let skill_filter = cx.new(|cx| TextInput::new("Filter skills", FILTER_LIMIT, cx));
        // The list below the field follows every keystroke.
        cx.observe(&skill_filter, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&skill_filter, |_, field, event: &FieldEvent, cx| {
            if let FieldEvent::Cancelled = event {
                field.update(cx, |field, cx| field.set_text("", cx));
            }
        })
        .detach();
        Self {
            commands,
            tray,
            tab: Tab::Overview,
            page: Page::General,
            known: false,
            status: None,
            sessions: Vec::new(),
            requests: Vec::new(),
            announced: HashSet::new(),
            label,
            label_for: None,
            focus_label: false,
            agent: None,
            browsers: Vec::new(),
            unpacked: Unpacked::default(),
            notices: NoticeBook::default(),
            confirmations: Vec::new(),
            code: None,
            code_seq: 0,
            unpair_all: false,
            line: None,
            line_seq: 0,
            pending: None,
            pending_seq: 0,
            settings,
            preflight: None,
            first_run: false,
            first_run_asked: false,
            fixing: false,
            npm: true,
            preferences: None,
            skills: Vec::new(),
            skill_filter,
            skill_source: None,
            approvals: Vec::new(),
            downloads: Vec::new(),
            log: LogTail::default(),
            log_scroll: ScrollHandle::new(),
            log_polling: false,
            command_line: CommandLine::default(),
            login: None,
            uninstall: false,
            keep_skills: true,
            update: UpdateState::default(),
            macos: String::new(),
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn running(&self) -> bool {
        self.status.is_some()
    }

    /// Marks a control busy until its answer arrives, or until its limit passes.
    fn hold(&mut self, pending: Pending, cx: &mut Context<Self>) {
        self.pending = Some(pending);
        self.pending_seq += 1;
        let seq = self.pending_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(pending.limit()).await;
            let _ = this.update(cx, |view, cx| {
                if view.pending_seq == seq {
                    view.pending = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_power(&mut self, cx: &mut Context<Self>) {
        if !self.known || self.pending.is_some() {
            return;
        }
        let (command, pending) = if self.running() {
            (Command::Stop, Pending::Stop)
        } else {
            (Command::Start, Pending::Start)
        };
        self.send(command);
        self.hold(pending, cx);
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        if !self.running() || self.pending.is_some() {
            return;
        }
        self.send(Command::Restart);
        self.hold(Pending::Restart, cx);
    }

    fn decide_first(&mut self, allow: bool, cx: &mut Context<Self>) {
        if self.pending == Some(Pending::Decide) {
            return;
        }
        let Some(request) = self.requests.first() else {
            return;
        };
        let label = if allow {
            self.label.read(cx).text().trim().to_owned()
        } else {
            String::new()
        };
        self.send(Command::Decide {
            request: request.id.clone(),
            allow,
            label,
        });
        self.hold(Pending::Decide, cx);
    }

    fn confirm(&mut self, id: String, allow: bool, remember: bool, cx: &mut Context<Self>) {
        if self.pending == Some(Pending::Confirm) {
            return;
        }
        self.send(Command::Confirm {
            id,
            allow,
            remember,
        });
        self.hold(Pending::Confirm, cx);
    }

    /// Runs first-run fixes; the checks run again once they finish.
    fn fix(&mut self, fixes: Vec<Fix>, cx: &mut Context<Self>) {
        if self.fixing || fixes.is_empty() {
            return;
        }
        self.fixing = true;
        self.send(Command::Fixes(fixes));
        cx.notify();
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        self.preflight = Some(Preflight::checking());
        self.send(Command::Recheck);
        cx.notify();
    }

    /// About's "Run the checks again": the first-run screen, until Continue.
    fn run_checks_again(&mut self, cx: &mut Context<Self>) {
        self.first_run = true;
        self.first_run_asked = true;
        self.recheck(cx);
    }

    fn say(&mut self, text: String, error: bool, cx: &mut Context<Self>) {
        self.say_for(text, error, LINE_LIFE, cx);
    }

    fn say_for(&mut self, text: String, error: bool, life: Duration, cx: &mut Context<Self>) {
        self.line = Some(Line { text, error });
        self.line_seq += 1;
        let seq = self.line_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(life).await;
            let _ = this.update(cx, |view, cx| {
                if view.line_seq == seq {
                    view.line = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.say("Copied.".to_owned(), false, cx);
    }

    /// Changes the app's own settings and writes `desktop.json`.
    fn save(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        match settings::edit(change) {
            Ok(now) => self.settings = now,
            Err(error) => self.say(format!("Could not save desktop.json: {error}"), true, cx),
        }
        cx.notify();
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.set_global(theme::Chosen(appearance));
        theme::paint(window, cx);
        window.refresh();
        self.save(|settings| settings.appearance = appearance, cx);
    }

    fn set_login(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Err(error) = login_item::set(on) {
            self.say(error, true, cx);
        }
        self.login = login_item::state();
        cx.notify();
    }

    fn set_preference(&self, change: Value) {
        self.send(Command::SetPreference(change));
    }

    fn receive(&mut self, message: Inbound, cx: &mut Context<Self>) {
        match message {
            Inbound::Menu(Pick::Power) => self.toggle_power(cx),
            Inbound::Menu(Pick::Restart) => self.restart(cx),
            Inbound::Menu(Pick::Open) => {
                tray::reveal();
                cx.activate(true);
            }
            Inbound::Menu(Pick::Quit) => cx.quit(),
            Inbound::Bridge(update) => self.apply(update, cx),
        }
    }

    fn apply(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Disconnected => {
                self.known = true;
                self.status = None;
                self.sessions.clear();
                self.requests.clear();
                self.confirmations.clear();
                self.agent = None;
                self.code = None;
                self.unpair_all = false;
                if self.pending == Some(Pending::Stop) {
                    self.pending = None;
                }
            }
            Update::Starting => {
                self.known = true;
                self.hold(Pending::Start, cx);
            }
            Update::Status(status) => {
                self.known = true;
                self.status = Some(status);
                if self.pending == Some(Pending::Start) {
                    self.pending = None;
                }
            }
            Update::Sessions(rows) => self.sessions = sessions(&rows),
            Update::PairRequests(value) => {
                self.requests = pair_requests(&value);
                if self.pending == Some(Pending::Decide) {
                    self.pending = None;
                }
                let first = self.requests.first();
                if first.map(|request| &request.id) != self.label_for.as_ref() {
                    self.label_for = first.map(|request| request.id.clone());
                    let suggested = first
                        .map(|request| request.suggested_label.clone())
                        .unwrap_or_default();
                    self.label
                        .update(cx, |input, cx| input.set_text(suggested, cx));
                    self.focus_label = first.is_some();
                }
                self.announce_requests(cx);
            }
            Update::Agent(state) => self.agent = agent_card(&state),
            Update::Browsers(value) => {
                self.browsers = browser_rows(&value);
                self.unpacked = unpacked(&value);
            }
            Update::Notices(value) => {
                for notice in self.notices.take(notices(&value)) {
                    notify(cx, notice.title, notice.body);
                }
            }
            Update::Confirmations(value) => {
                self.confirmations = confirmations(&value);
                if self.pending == Some(Pending::Confirm) {
                    self.pending = None;
                }
                self.announce_confirmations(cx);
            }
            Update::PairCode { code, expires_at } => {
                self.code = Some((pairing_code(&code), expires_at));
                self.count_down(cx);
            }
            Update::Preflight { checks, npm } => {
                self.npm = npm;
                self.fixing = false;
                // A failing required check holds the window; passing ones let it go on, unless
                // the screen was asked for from About.
                if !checks.ready() {
                    self.first_run = true;
                } else if !self.first_run_asked {
                    self.first_run = false;
                }
                self.preflight = Some(checks);
            }
            Update::Preferences(value) => self.preferences = Some(preferences(&value)),
            Update::Skills(value) => self.skills = skill_rows(&value),
            Update::Activity {
                approvals,
                downloads,
            } => {
                self.approvals = approval_hosts(&approvals);
                self.downloads = download_rows(&downloads);
            }
            Update::CommandLine(found) => self.command_line = found,
            Update::Unpacked { dir, said } => {
                cx.write_to_clipboard(ClipboardItem::new_string(dir));
                self.say_for(said, false, STEPS_LIFE, cx);
            }
            Update::UpdateState(state) => self.update = state,
            Update::MacOs(version) => self.macos = version,
            Update::Quit => cx.quit(),
            Update::Said(text) => {
                if self.pending == Some(Pending::Restart) {
                    self.pending = None;
                }
                self.say(text, false, cx);
            }
            Update::Error(text) => {
                self.pending = None;
                self.fixing = false;
                self.say(text, true, cx);
            }
        }
        if let Some(tray) = &self.tray {
            tray.show(&self.tray_line(), self.running());
        }
        cx.notify();
    }

    /// A browser asking to connect waits on this app, so say so even with the window hidden.
    fn announce_requests(&mut self, cx: &mut Context<Self>) {
        let fresh: Vec<String> = self
            .requests
            .iter()
            .filter(|request| self.announced.insert(request.id.clone()))
            .map(|request| request.name.clone())
            .collect();
        for name in fresh {
            notify(
                cx,
                format!("{name} wants to connect to Vexdom"),
                "Open Vexdom to allow it.".to_owned(),
            );
        }
    }

    /// An action waiting for approval holds up an agent: say so even with the window hidden.
    fn announce_confirmations(&mut self, cx: &mut Context<Self>) {
        let fresh: Vec<(String, String)> = self
            .confirmations
            .iter()
            .filter(|confirmation| self.announced.insert(confirmation.id.clone()))
            .map(|confirmation| (confirmation.question(), confirmation.summary.clone()))
            .collect();
        for (question, summary) in fresh {
            let body = if summary.is_empty() {
                "Open Vexdom to answer.".to_owned()
            } else {
                summary
            };
            notify(cx, question, body);
        }
    }

    fn tray_line(&self) -> String {
        if let Some(request) = self.requests.first() {
            return format!("{} wants to connect", request.name);
        }
        match self.confirmations.len() {
            0 => self.headline().title,
            1 => "An action waits for your approval".to_owned(),
            count => format!("{count} actions wait for your approval"),
        }
    }

    /// The headline, with a waiting request named in it.
    fn headline(&self) -> Headline {
        if !self.known {
            return Headline {
                lamp: Lamp::Off,
                caption: "Checking",
                title: "Looking for the Bridge".to_owned(),
            };
        }
        let mut headline = headline(self.status.as_ref());
        if headline.lamp == Lamp::Waiting
            && let Some(request) = self.requests.first()
        {
            headline.title = format!("{} wants to connect", request.name);
        }
        headline
    }

    pub fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        match tab {
            Tab::Browsers | Tab::Agent => self.send(Command::Refresh),
            Tab::Settings => self.show_page(self.page, cx),
            Tab::Overview | Tab::About => {}
        }
        cx.notify();
    }

    fn show_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        match page {
            Page::General => {
                self.send(Command::Preferences);
                self.login = login_item::state();
            }
            Page::Skills => self.send(Command::Skills),
            Page::Activity => self.send(Command::Activity),
            Page::Logs => self.watch_log(cx),
        }
        cx.notify();
    }

    /// Reads the log every second while Settings → Logs shows.
    fn watch_log(&mut self, cx: &mut Context<Self>) {
        self.poll_log(true);
        if std::mem::replace(&mut self.log_polling, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LOG_POLL).await;
                let going = this.update(cx, |view, cx| {
                    let shown = view.tab == Tab::Settings && view.page == Page::Logs;
                    if !shown {
                        view.log_polling = false;
                    } else if view.poll_log(false) {
                        cx.notify();
                    }
                    shown
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Reads what the log gained; the list keeps to its end unless the reader scrolled up.
    /// `jump` takes it to the end regardless (the page was just opened).
    fn poll_log(&mut self, jump: bool) -> bool {
        let scrolled = -f32::from(self.log_scroll.offset().y);
        let max = f32::from(self.log_scroll.max_offset().height);
        let following = jump || at_bottom(scrolled, max);
        let changed = self.log.poll(&vexdom_core::paths::log_path());
        if following && (changed || jump) {
            self.log_scroll.scroll_to_bottom();
        }
        changed
    }

    /// Re-renders the pairing code's countdown every second, and drops the code when it runs out.
    fn count_down(&mut self, cx: &mut Context<Self>) {
        self.code_seq += 1;
        let seq = self.code_seq;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let going = this.update(cx, |view, cx| {
                    if view.code_seq != seq {
                        return false;
                    }
                    let live = view
                        .code
                        .as_ref()
                        .is_some_and(|(_, at)| countdown(*at, now_ms()).is_some());
                    if !live {
                        view.code = None;
                    }
                    cx.notify();
                    live
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn tab_button(&self, tab: Tab, label: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = theme::palette(cx);
        let chosen = self.tab == tab;
        // A browser asking to connect, or an action waiting for approval, is answered on
        // Overview: mark the tab from elsewhere.
        let asking = tab == Tab::Overview
            && !chosen
            && (!self.requests.is_empty() || !self.confirmations.is_empty());
        let base = div()
            .id(label)
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .py(px(3.))
            .rounded(px(5.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .child(label)
            .when(asking, |tab| tab.child(lamp(6., p.lamp_waiting)))
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.show_tab(tab, cx)));
        if chosen {
            base.bg(p.face).text_color(p.ink).shadow(p.lift())
        } else {
            base.text_color(p.label)
                .hover(move |style| style.text_color(p.ink))
        }
    }

    /// The signature: a round push switch in its well. Raised and bright while off; latched in
    /// (smaller, duller, no lift) while the Bridge runs, its well ringed in the lamp's colour.
    /// Dimmed and inert while it is not known yet or a Start, Stop or Restart is under way.
    fn power_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::palette(cx);
        let running = self.running();
        let busy = !self.known
            || matches!(
                self.pending,
                Some(Pending::Start | Pending::Stop | Pending::Restart)
            );
        let label = match self.pending {
            Some(Pending::Start) => "Starting",
            Some(Pending::Stop) => "Stopping",
            Some(Pending::Restart) => "Restarting",
            _ if running => "Stop",
            _ => "Start",
        };
        let knob = div()
            .id("power")
            .size(px(if running { 104. } else { 112. }))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(19.))
            .font_weight(FontWeight::BOLD)
            .text_color(p.on_power)
            .child(label)
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.toggle_power(cx)));
        let knob = if busy {
            knob.bg(if running { p.power_latched } else { p.power })
                .opacity(0.6)
        } else if running {
            knob.cursor_pointer()
                .bg(p.power_latched)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.18),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(1.),
                    spread_radius: px(0.),
                }])
                .hover(move |style| style.bg(p.power_pressed))
        } else {
            knob.cursor_pointer()
                .bg(p.power)
                .shadow(vec![
                    gpui::BoxShadow {
                        color: gpui::black().opacity(0.16),
                        offset: gpui::point(px(0.), px(2.)),
                        blur_radius: px(3.),
                        spread_radius: px(0.),
                    },
                    gpui::BoxShadow {
                        color: gpui::black().opacity(0.12),
                        offset: gpui::point(px(0.), px(8.)),
                        blur_radius: px(18.),
                        spread_radius: px(-6.),
                    },
                ])
                .hover(move |style| style.bg(p.power_hover))
                .active(move |style| style.bg(p.power_pressed))
        };
        let ring = if running && !busy {
            lamp_color(&p, self.headline().lamp)
        } else {
            p.well_hover
        };
        div()
            .size(px(168.))
            .flex_none()
            .rounded_full()
            .bg(p.well)
            .border_2()
            .border_color(ring)
            .flex()
            .items_center()
            .justify_center()
            .child(knob)
    }

    fn overview(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let Headline {
            lamp: state,
            caption,
            title,
        } = self.headline();
        let mut fact_list = div().flex().flex_col().gap(px(6.)).mt(px(14.));
        for (name, value) in facts(self.status.as_ref(), self.agent.as_ref(), &self.sessions) {
            let value = div().text_color(p.ink).child(value);
            fact_list = fact_list.child(
                div()
                    .flex()
                    .gap_4()
                    .text_size(px(13.))
                    .child(div().w(px(64.)).flex_none().text_color(p.label).child(name))
                    .child(if name == "Address" {
                        value.font(p.figures())
                    } else {
                        value
                    }),
            );
        }
        let restarting = self.pending == Some(Pending::Restart);
        let restart = div().flex().mt(px(6.)).child(
            p.button(
                "restart",
                if restarting { "Restarting" } else { "Restart" },
                Tone::Secondary,
            )
            .when(self.pending.is_some(), |button| button.opacity(0.5))
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.restart(cx))),
        );
        let state_face = p
            .panel()
            .flex_row()
            .items_center()
            .gap(px(28.))
            .p(px(24.))
            .child(self.power_switch(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(13.))
                            .text_color(p.label)
                            .child(lamp(10., lamp_color(&p, state)))
                            .child(caption),
                    )
                    .child(
                        div()
                            .mt(px(8.))
                            .text_size(px(28.))
                            .line_height(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.ink)
                            .child(title),
                    )
                    .child(if self.running() {
                        fact_list.child(restart)
                    } else if !self.known {
                        div()
                    } else {
                        quiet(
                            &p,
                            "Press Start to run the Bridge. Your browsers and the agent show here once it runs.",
                        )
                        .mt(px(10.))
                    }),
            );
        let mut column = div().flex().flex_col().gap(px(14.)).child(state_face);
        if let Some(request) = self.requests.first() {
            column = column.child(self.request_face(request, cx));
        }
        if !self.confirmations.is_empty() {
            column = column.child(self.confirmation_face(cx));
        }
        column.child(self.notice_list(cx))
    }

    fn request_face(&self, request: &PairRequest, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let waiting = self.requests.len() - 1;
        let detail = if waiting == 0 {
            "Your agent can then use this browser.".to_owned()
        } else {
            format!("{waiting} more waiting after this one.")
        };
        let busy = self.pending == Some(Pending::Decide);
        p.panel()
            .flex_row()
            .items_center()
            .gap_3()
            .px(px(20.))
            .py(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(heading(&p, format!("Allow {}?", request.name)))
                    .child(quiet(&p, detail)),
            )
            .child(self.label.clone())
            .child(
                p.button("deny", "Deny", Tone::Secondary)
                    .when(busy, |button| button.opacity(0.5))
                    .on_click(
                        cx.listener(|view, _: &ClickEvent, _, cx| view.decide_first(false, cx)),
                    ),
            )
            .child(
                p.button("allow", "Allow", Tone::Primary)
                    .when(busy, |button| button.opacity(0.5))
                    .on_click(
                        cx.listener(|view, _: &ClickEvent, _, cx| view.decide_first(true, cx)),
                    ),
            )
    }

    /// Actions from an MCP client or the CLI that a guardrail holds for the user (ADR-032).
    fn confirmation_face(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let busy = self.pending == Some(Pending::Confirm);
        let entries = self
            .confirmations
            .iter()
            .map(|confirmation| {
                let answer = |allow: bool, remember: bool| {
                    let id = confirmation.id.clone();
                    cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.confirm(id.clone(), allow, remember, cx);
                    })
                };
                let id = &confirmation.id;
                let mut actions = div().flex().gap_2().child(
                    p.button(
                        SharedString::from(format!("deny-{id}")),
                        "Deny",
                        Tone::Secondary,
                    )
                    .on_click(answer(false, false)),
                );
                if !confirmation.site.is_empty() {
                    actions = actions.child(
                        p.button(
                            SharedString::from(format!("always-{id}")),
                            format!("Always on {}", confirmation.site),
                            Tone::Secondary,
                        )
                        .on_click(answer(true, true)),
                    );
                }
                actions = actions.child(
                    p.button(
                        SharedString::from(format!("allow-{id}")),
                        "Allow",
                        Tone::Primary,
                    )
                    .on_click(answer(true, false)),
                );
                let detail = if confirmation.summary.is_empty() {
                    "An agent waits for your answer.".to_owned()
                } else {
                    confirmation.summary.clone()
                };
                row(
                    confirmation.question(),
                    quiet(&p, detail),
                    actions
                        .when(busy, |actions| actions.opacity(0.5))
                        .into_any_element(),
                )
            })
            .collect();
        list_face(&p, "Waiting for your approval", entries, "")
    }

    fn notice_list(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        if self.notices.shown.is_empty() {
            return div()
                .px(px(4.))
                .child(quiet(&p, "Notices from tasks and monitors show up here."));
        }
        let entries = self
            .notices
            .shown
            .iter()
            .map(|notice| {
                let id = notice.id.clone();
                row(
                    notice.title.clone(),
                    quiet(&p, notice.body.clone()),
                    p.button(
                        SharedString::from(format!("open-{id}")),
                        "Open",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::OpenNotice(id.clone()));
                    }))
                    .into_any_element(),
                )
            })
            .collect();
        list_face(&p, "Notices", entries, "")
    }

    /// How browsers pair: the mode, a code for one that asks for it, and Unpair all.
    fn pairing_face(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let face = p.panel().gap(px(12.)).px(px(20.)).py(px(16.));
        if !self.running() {
            return face
                .child(heading(&p, "Pairing"))
                .child(quiet(&p, "Start Vexdom to pair a browser."));
        }
        let mode = self
            .status
            .as_ref()
            .and_then(|status| status.pointer("/pairing/mode"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut modes = track(&p);
        for (value, label) in PAIRING_MODES {
            modes = modes.child(
                chip(
                    &p,
                    SharedString::from(format!("pairing-{value}")),
                    label,
                    mode == value,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                    view.send(Command::SetPairing(value.to_owned()));
                })),
            );
        }
        let live = self
            .code
            .as_ref()
            .and_then(|(code, at)| countdown(*at, now_ms()).map(|left| (code.clone(), left)));
        let code = match live {
            Some((code, left)) => div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .font(p.figures())
                        .text_size(px(22.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.ink)
                        .child(code),
                )
                .child(quiet(
                    &p,
                    format!("Type it into the extension. It works for {left}."),
                )),
            None => div().flex().child(
                p.button("pair-code", "Get a pairing code", Tone::Secondary)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                        view.send(Command::PairCode);
                    })),
            ),
        };
        let unpair = if self.unpair_all {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    quiet(&p, "Unpair every browser? Each one has to pair again.")
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    p.button("unpair-all-cancel", "Cancel", Tone::Secondary)
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.unpair_all = false;
                            cx.notify();
                        })),
                )
                .child(
                    p.button("unpair-all-confirm", "Unpair all", Tone::Primary)
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.unpair_all = false;
                            view.send(Command::RevokeAll);
                            cx.notify();
                        })),
                )
        } else if self.sessions.is_empty() {
            div()
        } else {
            div().flex().child(
                p.button("unpair-all", "Unpair all", Tone::Secondary)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.unpair_all = true;
                        cx.notify();
                    })),
            )
        };
        face.child(heading(&p, "Pairing"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(modes)
                    .child(quiet(&p, pairing_line(mode))),
            )
            .child(code)
            .child(unpair)
    }

    fn browsers_tab(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let installed: Vec<Div> = self
            .browsers
            .iter()
            .map(|browser| {
                let id = browser.id.clone();
                // The live session list, not the CLI's snapshot: it changes while the tab is open.
                let own = || {
                    self.sessions
                        .iter()
                        .filter(|session| session.browser == browser.label)
                };
                let connected = own().any(|session| session.connected);
                let stale = own().any(|session| reload_needed(session, &self.unpacked));
                let (state, text) = match (self.running(), connected, own().count()) {
                    (false, ..) => (Lamp::Off, "Start Vexdom to see its state".to_owned()),
                    (true, true, _) => (Lamp::Connected, "Connected".to_owned()),
                    (true, false, 0) => (Lamp::Off, "Extension not paired".to_owned()),
                    (true, false, 1) => (Lamp::Waiting, "Paired, not open".to_owned()),
                    (true, false, n) => (Lamp::Waiting, format!("{n} pairings, none open")),
                };
                let action = if connected {
                    div().into_any_element()
                } else if browser.source == "unpacked" {
                    // Every Chromium browser takes the unpacked folder: copy it, open the page.
                    p.button(
                        SharedString::from(format!("unpacked-{id}")),
                        "Load it unpacked",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::LoadUnpacked {
                            browser: id.clone(),
                        });
                    }))
                    .into_any_element()
                } else {
                    p.button(
                        SharedString::from(format!("add-{id}")),
                        "Add extension",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::InstallFor {
                            browser: id.clone(),
                        });
                    }))
                    .into_any_element()
                };
                row(
                    browser.label.clone(),
                    lamp_line(&p, state, text, p.label)
                        .when(stale, |line| line.child(badge(&p, "Reload needed"))),
                    action,
                )
            })
            .collect();
        let paired: Vec<Div> = self
            .sessions
            .iter()
            .map(|session| {
                let id = session.id.clone();
                let stale = reload_needed(session, &self.unpacked);
                let page = self
                    .browsers
                    .iter()
                    .find(|browser| browser.label == session.browser)
                    .map_or("its extensions page", |browser| {
                        browser.extensions_page.as_str()
                    });
                let detail = if stale {
                    format!(
                        "{} · runs v{}; reload it at {page}",
                        session.detail, session.extension_version
                    )
                } else {
                    session.detail.clone()
                };
                row(
                    session.title.clone(),
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(quiet(&p, detail))
                        .when(stale, |line| line.child(badge(&p, "Reload needed"))),
                    p.button(
                        SharedString::from(format!("unpair-{id}")),
                        "Unpair",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::Revoke {
                            session: id.clone(),
                        });
                    }))
                    .into_any_element(),
                )
            })
            .collect();
        let none_paired = if self.running() {
            "No browser is paired yet. Add the extension, then allow it on Overview."
        } else {
            "Start Vexdom to see paired browsers."
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.pairing_face(cx))
            .child(list_face(
                &p,
                "Browsers on this Mac",
                installed,
                "No supported browser found.",
            ))
            .child(list_face(&p, "Paired", paired, none_paired))
    }

    fn agent_tab(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let Some(agent) = &self.agent else {
            return p
                .panel()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(20.))
                .py(px(18.))
                .child(quiet(&p, "Start Vexdom to see Claude Code and its model."))
                .child(
                    p.button("agent-start", "Start Vexdom", Tone::Primary)
                        .on_click(
                            cx.listener(|view, _: &ClickEvent, _, _| view.send(Command::Start)),
                        ),
                );
        };
        let mut picker = div().flex().flex_wrap().gap_2();
        let choices = std::iter::once((String::new(), "Default".to_owned())).chain(
            agent
                .models
                .iter()
                .map(|model| (model.clone(), model.clone())),
        );
        for (model, label) in choices {
            let tone = if agent.model == model {
                Tone::Primary
            } else {
                Tone::Secondary
            };
            picker = picker.child(
                p.button(SharedString::from(format!("model-{label}")), label, tone)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::SetAgent {
                            model: (!model.is_empty()).then(|| model.clone()),
                        });
                    })),
            );
        }
        let (state, status) = if agent.ready {
            (
                Lamp::Connected,
                format!("Ready · version {}", short_version(&agent.detail)),
            )
        } else {
            (Lamp::Off, agent.detail.clone())
        };
        let remedy = if agent.ready {
            div()
        } else if agent.problem == AGENT_MISSING {
            let install = p
                .button(
                    "install-claude",
                    if self.fixing {
                        "Installing…"
                    } else {
                        "Install Claude Code"
                    },
                    Tone::Primary,
                )
                .when(self.fixing || !self.npm, |button| button.opacity(0.5));
            let install = if self.npm {
                install.on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.fix(vec![Fix::InstallClaude { npm: true }], cx);
                }))
            } else {
                install
            };
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(install)
                .when(!self.npm, |line| line.child(quiet(&p, "npm not found")))
        } else {
            div().flex().items_center().gap_3().child(
                p.button("fix-agent", "Fix", Tone::Primary)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                        view.send(Command::FixAgent);
                    })),
            )
        };
        p.panel()
            .gap(px(16.))
            .px(px(20.))
            .py(px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(heading(&p, "Claude Code"))
                    .child(lamp_line(
                        &p,
                        state,
                        status,
                        if agent.ready { p.label } else { p.fault },
                    ))
                    .when(!agent.ready && !agent.fix.is_empty(), |card| {
                        card.child(quiet(&p, agent.fix.clone()))
                    }),
            )
            .child(remedy)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(quiet(&p, "Model"))
                    .child(picker),
            )
    }

    fn settings_tab(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let mut nav = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(124.))
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(p.well_hover);
        for (page, label) in Page::ALL {
            let item = div()
                .id(SharedString::from(format!("page-{label}")))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(7.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.show_page(page, cx)));
            let ink = p.ink;
            nav = nav.child(if self.page == page {
                item.bg(p.face).text_color(p.ink).shadow(p.lift())
            } else {
                item.text_color(p.label)
                    .hover(move |style| style.text_color(ink))
            });
        }
        let content = match self.page {
            Page::General => self.general_page(cx),
            Page::Skills => self.skills_page(cx),
            Page::Activity => self.activity_page(cx),
            Page::Logs => self.logs_page(cx),
        };
        div()
            .flex()
            .items_start()
            .gap(px(14.))
            .child(nav)
            .child(div().flex_1().min_w_0().child(content))
    }

    fn general_page(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let mut looks = track(&p);
        for appearance in Appearance::ALL {
            looks = looks.child(
                chip(
                    &p,
                    SharedString::from(format!("appearance-{}", appearance.id())),
                    appearance.label(),
                    self.settings.appearance == appearance,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                    view.set_appearance(appearance, window, cx);
                })),
            );
        }
        let start = self.settings.start_bridge_on_launch;
        let mut app_rows = vec![
            row(
                "Appearance",
                quiet(&p, "Light or dark, or as macOS shows it."),
                looks.into_any_element(),
            ),
            row(
                "Start Vexdom when the app opens",
                quiet(&p, "Runs the Bridge as soon as this app opens."),
                switch(&p, "start-on-launch", start)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.save(|settings| settings.start_bridge_on_launch = !start, cx);
                    }))
                    .into_any_element(),
            ),
        ];
        if let Some(login) = self.login {
            let on = login != LoginState::Off;
            let detail = if login == LoginState::NeedsApproval {
                "Allow Vexdom in System Settings → General → Login Items."
            } else {
                "Vexdom opens in the menu bar when you log in."
            };
            app_rows.push(row(
                "Open at login",
                quiet(&p, detail),
                switch(&p, "open-at-login", on)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.set_login(!on, cx);
                    }))
                    .into_any_element(),
            ));
        }
        let mut page = div().flex().flex_col().gap(px(14.)).child(
            p.panel()
                .px(px(20.))
                .py(px(8.))
                .child(heading(&p, "This app").py(px(6.)))
                .child(rows(&p, app_rows)),
        );
        page = match &self.preferences {
            Some(prefs) => page
                .child(self.theme_face(prefs, cx))
                .child(self.guardrails_face(prefs, cx)),
            None => page.child(
                p.panel()
                    .px(px(20.))
                    .py(px(16.))
                    .child(quiet(&p, "Reading the browser's settings…")),
            ),
        };
        page.child(self.command_line_face(cx))
            .child(self.uninstall_face(cx))
    }

    fn theme_face(&self, prefs: &Preferences, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let mut themes = div().flex().flex_wrap().gap_2();
        for id in vexdom_core::config::THEMES {
            let tone = if prefs.theme == id {
                Tone::Primary
            } else {
                Tone::Secondary
            };
            themes = themes.child(
                p.button(
                    SharedString::from(format!("theme-{id}")),
                    title_case(id),
                    tone,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                    view.set_preference(json!({"kind": "theme", "theme": id}));
                })),
            );
        }
        p.panel()
            .gap(px(10.))
            .px(px(20.))
            .py(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(heading(&p, "Browser theme"))
                    .child(quiet(&p, "How the extension's side panel looks.")),
            )
            .child(themes)
    }

    fn guardrails_face(&self, prefs: &Preferences, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let mut entries: Vec<Div> = prefs
            .rules
            .iter()
            .map(|rule| {
                let action = if rule.locked {
                    quiet(&p, "Locked").into_any_element()
                } else {
                    let mut choices = track(&p);
                    for (effect, label) in EFFECTS {
                        let change = json!({
                            "kind": "guardrail",
                            "setting": rule.id,
                            "value": rule.value_for(effect),
                        });
                        choices = choices.child(
                            chip(
                                &p,
                                SharedString::from(format!("rule-{}-{effect}", rule.id)),
                                label,
                                rule.effect == effect,
                            )
                            .on_click(cx.listener(
                                move |view, _: &ClickEvent, _, _| {
                                    view.set_preference(change.clone());
                                },
                            )),
                        );
                    }
                    choices.into_any_element()
                };
                row(rule.title.clone(), quiet(&p, rule.reason.clone()), action)
            })
            .collect();
        let fence = prefs.fence;
        entries.push(row(
            "Fence page text",
            quiet(&p, "Page text reaches the agent marked as untrusted."),
            switch(&p, "fence", fence)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                    // On is the default: written as null.
                    let value = if fence { json!(false) } else { Value::Null };
                    view.set_preference(
                        json!({"kind": "guardrail", "setting": "fence", "value": value}),
                    );
                }))
                .into_any_element(),
        ));
        let mut unattended = track(&p);
        for (allow, label) in [(false, "Refuse"), (true, "Allow")] {
            unattended = unattended.child(
                chip(
                    &p,
                    SharedString::from(format!("unattended-{label}")),
                    label,
                    prefs.unattended_allow == allow,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                    // Refusing is the default: written as null.
                    let value = if allow { json!("allow") } else { Value::Null };
                    view.set_preference(
                        json!({"kind": "guardrail", "setting": "unattended", "value": value}),
                    );
                })),
            );
        }
        entries.push(row(
            "When nobody can answer",
            quiet(&p, "An action that asks, while nobody is there to say yes."),
            unattended.into_any_element(),
        ));
        list_face(&p, "Guardrails", entries, "")
    }

    fn command_line_face(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let found = &self.command_line;
        let place = match (&found.linked, &found.bundled, &found.vexdom) {
            (Some(dir), _, _) => format!("Linked as {}.", dir.join("vexdom").display()),
            (None, Some(_), _) => {
                "Not linked: a terminal does not find this app's vexdom.".to_owned()
            }
            (None, None, Some(path)) => format!("Runs {}.", path.display()),
            (None, None, None) => "There is no vexdom beside this app or on your PATH.".to_owned(),
        };
        let link = match (&found.bundled, &found.linked) {
            (Some(_), Some(_)) => p
                .button("unlink", "Unlink", Tone::Secondary)
                .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                    view.send(Command::Link(false));
                }))
                .into_any_element(),
            (Some(_), None) => p
                .button("link", "Link vexdom", Tone::Primary)
                .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                    view.send(Command::Link(true));
                }))
                .into_any_element(),
            (None, _) => div().into_any_element(),
        };
        let mut entries = vec![row("vexdom", quiet(&p, place), link)];
        let path = found
            .linked
            .as_ref()
            .map(|dir| dir.join("vexdom"))
            .or_else(|| found.vexdom.clone());
        if let Some(path) = path {
            let snippet = mcp_command(&path.to_string_lossy());
            let copied = snippet.clone();
            entries.push(row(
                "Claude Code",
                div()
                    .font_family(theme::MONO)
                    .text_size(px(12.))
                    .text_color(p.label)
                    .child(snippet),
                p.button("copy-mcp", "Copy", Tone::Secondary)
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.copy(copied.clone(), cx);
                    }))
                    .into_any_element(),
            ));
        }
        list_face(&p, "Command line", entries, "")
    }

    fn uninstall_face(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let face = p.panel().px(px(20.)).py(px(16.));
        if !self.uninstall {
            return face
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .min_w_0()
                        .child(heading(&p, "Uninstall"))
                        .child(quiet(
                            &p,
                            "Removes the Bridge's state and the browsers' wake-up from this Mac.",
                        )),
                )
                .child(
                    p.button("uninstall", "Uninstall…", Tone::Secondary)
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                            view.uninstall = true;
                            cx.notify();
                        })),
                );
        }
        let keep = self.keep_skills;
        face.gap(px(12.))
            .child(heading(&p, "Uninstall Vexdom?"))
            .child(quiet(
                &p,
                "The Bridge stops; its pairings, settings, approvals and logs are deleted, and vexdom is unlinked. Vexdom.app stays until you move it to the Bin.",
            ))
            .child(div().flex().child(
                checkbox(&p, "keep-skills", keep, "Keep my skills").on_click(cx.listener(
                    move |view, _: &ClickEvent, _, cx| {
                        view.keep_skills = !keep;
                        cx.notify();
                    },
                )),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        p.button("uninstall-cancel", "Cancel", Tone::Secondary)
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                view.uninstall = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        p.button("uninstall-confirm", "Uninstall", Tone::Primary)
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                view.uninstall = false;
                                view.send(Command::Uninstall { keep_skills: keep });
                                // Back to the first-run screen, which waits for the checks.
                                view.first_run = true;
                                view.first_run_asked = false;
                                view.preflight = Some(Preflight::checking());
                                cx.notify();
                            })),
                    ),
            )
    }

    fn skills_page(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let mut sources = track(&p).child(
            chip(&p, "source-all", "All", self.skill_source.is_none()).on_click(cx.listener(
                |view, _: &ClickEvent, _, cx| {
                    view.skill_source = None;
                    cx.notify();
                },
            )),
        );
        for source in SKILL_SOURCES {
            sources = sources.child(
                chip(
                    &p,
                    SharedString::from(format!("source-{source}")),
                    title_case(source),
                    self.skill_source == Some(source),
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.skill_source = Some(source);
                    cx.notify();
                })),
            );
        }
        let text = self.skill_filter.read(cx).text().to_owned();
        let entries: Vec<Div> = filter_skills(&self.skills, &text, self.skill_source)
            .into_iter()
            .map(|skill| {
                row(
                    skill.name.clone(),
                    quiet(&p, skill.description.clone()),
                    quiet(&p, title_case(&skill.source)).into_any_element(),
                )
            })
            .collect();
        let empty = if self.skills.is_empty() {
            "No skills found."
        } else {
            "No skill matches."
        };
        p.panel()
            .px(px(20.))
            .py(px(8.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .py(px(6.))
                    .child(heading(&p, "Skills"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.skill_filter.clone())
                            .child(sources),
                    ),
            )
            .child(if entries.is_empty() {
                quiet(&p, empty).py(px(10.))
            } else {
                rows(&p, entries)
            })
    }

    fn activity_page(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let approvals: Vec<Div> = self
            .approvals
            .iter()
            .map(|site| {
                let host = site.host.clone();
                row(
                    site.host.clone(),
                    quiet(&p, site.actions.join(", ")),
                    p.button(
                        SharedString::from(format!("forget-{host}")),
                        "Forget",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, _| {
                        view.send(Command::ForgetApprovals(host.clone()));
                    }))
                    .into_any_element(),
                )
            })
            .collect();
        let downloads: Vec<Div> = self
            .downloads
            .iter()
            .enumerate()
            .map(|(index, download)| {
                let path = download.path.clone();
                row(
                    download.name.clone(),
                    quiet(&p, download.detail.clone()),
                    p.button(
                        SharedString::from(format!("show-{index}")),
                        "Show",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.reveal_path(Path::new(&path));
                    }))
                    .into_any_element(),
                )
            })
            .collect();
        let clear = if self.downloads.is_empty() {
            div()
        } else {
            div().child(
                p.button("clear-downloads", "Clear all", Tone::Secondary)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                        view.send(Command::ClearDownloads);
                    })),
            )
        };
        let downloads_face = p
            .panel()
            .px(px(20.))
            .py(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(px(6.))
                    .child(heading(&p, "Downloads"))
                    .child(clear),
            )
            .child(if downloads.is_empty() {
                quiet(&p, "No file captured yet.").py(px(10.))
            } else {
                rows(&p, downloads)
            });
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(list_face(
                &p,
                "Standing approvals",
                approvals,
                "No standing approvals: every gated action still asks.",
            ))
            .child(downloads_face)
    }

    fn logs_page(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let lines: Vec<Div> = self
            .log
            .lines()
            .map(|line| div().child(line.to_owned()))
            .collect();
        p.panel()
            .gap(px(10.))
            .px(px(20.))
            .py(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(heading(&p, "Bridge log"))
                    .child(
                        quiet(&p, vexdom_core::paths::log_path().display().to_string())
                            .font_family(theme::MONO)
                            .text_size(px(11.)),
                    ),
            )
            .child(
                div()
                    .id("log")
                    .h(px(LOG_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.log_scroll)
                    .rounded(px(7.))
                    .bg(p.well)
                    .px(px(12.))
                    .py(px(10.))
                    .font_family(theme::MONO)
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(p.ink)
                    .when(self.log.is_empty(), |log| {
                        log.child(div().text_color(p.label).child("Nothing logged yet."))
                    })
                    .children(lines),
            )
    }

    fn about_tab(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let bridge = self
            .status
            .as_ref()
            .and_then(|status| status.get("daemonVersion"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let connected: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|session| session.connected && !session.extension_version.is_empty())
            .collect();
        let mut versions = vec![("Vexdom app".to_owned(), VERSION.to_owned())];
        if !bridge.is_empty() {
            versions.push(("Bridge".to_owned(), bridge.clone()));
        }
        for session in &connected {
            versions.push((
                format!("Extension in {}", session.title),
                session.extension_version.clone(),
            ));
        }
        let entries: Vec<Div> = versions
            .into_iter()
            .enumerate()
            .map(|(index, (name, version))| {
                let copied = version.clone();
                row(
                    name,
                    div()
                        .font(p.figures())
                        .text_size(px(13.))
                        .text_color(p.label)
                        .child(version),
                    p.button(
                        SharedString::from(format!("copy-{index}")),
                        "Copy",
                        Tone::Secondary,
                    )
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.copy(copied.clone(), cx);
                    }))
                    .into_any_element(),
                )
            })
            .collect();
        let extensions: Vec<String> = connected
            .iter()
            .map(|session| session.extension_version.clone())
            .collect();
        let report = bug_report_url(VERSION, &bridge, &extensions, &self.macos);
        let help = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                p.button("report-bug", "Report a bug", Tone::Secondary)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&report))),
            )
            .child(
                p.button("docs", "Docs", Tone::Secondary)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.open_url(DOCS))),
            )
            .child(
                p.button("release-notes", "Release notes", Tone::Secondary)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.open_url(RELEASES))),
            )
            .child(
                p.button("run-checks", "Run the checks again", Tone::Secondary)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.run_checks_again(cx);
                    })),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(list_face(&p, "Versions", entries, ""))
            .child(self.update_card(cx))
            .child(
                p.panel()
                    .gap(px(10.))
                    .px(px(20.))
                    .py(px(16.))
                    .child(heading(&p, "Help"))
                    .child(help),
            )
    }

    /// The app's own update: found at launch and every six hours, or when asked.
    fn update_card(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let (text, fault) = match &self.update {
            UpdateState::Idle => (
                "Vexdom looks for a new version when it opens and every six hours.".to_owned(),
                false,
            ),
            UpdateState::Checking => ("Looking for a new version…".to_owned(), false),
            UpdateState::Latest => (format!("Vexdom {VERSION} is the latest version."), false),
            UpdateState::Available(version) => (
                format!("Vexdom {version} is out; this is {VERSION}."),
                false,
            ),
            UpdateState::Downloading(version) => (format!("Downloading Vexdom {version}…"), false),
            UpdateState::Verifying(version) => (
                format!("Checking Vexdom {version}'s signature and bundle…"),
                false,
            ),
            UpdateState::Relaunching(version) => (
                format!("Vexdom {version} opens as soon as this one quits…"),
                false,
            ),
            UpdateState::Failed(message) => (message.clone(), true),
        };
        let action = match &self.update {
            UpdateState::Available(version) => p
                .button(
                    "install-update",
                    format!("Update to v{version}"),
                    Tone::Primary,
                )
                .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                    view.send(Command::InstallUpdate);
                }))
                .into_any_element(),
            UpdateState::Idle | UpdateState::Latest | UpdateState::Failed(_) => p
                .button("check-update", "Check for updates", Tone::Secondary)
                .on_click(cx.listener(|view, _: &ClickEvent, _, _| {
                    view.send(Command::CheckUpdate);
                }))
                .into_any_element(),
            UpdateState::Checking
            | UpdateState::Downloading(_)
            | UpdateState::Verifying(_)
            | UpdateState::Relaunching(_) => div().into_any_element(),
        };
        p.panel()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_4()
            .px(px(20.))
            .py(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(heading(&p, "Updates"))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(if fault { p.fault } else { p.label })
                            .child(text),
                    ),
            )
            .child(action)
    }

    /// One first-run check: its lamp, what was found, and the fix when it failed.
    fn check_row(&self, check: &Check, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let required = check.id.required();
        let (state, text, color) = match &check.outcome {
            Outcome::Checking => (Lamp::Off, "Checking…".to_owned(), p.label),
            Outcome::Pass(found) => (Lamp::Connected, found.clone(), p.label),
            Outcome::Fail(why) => (
                Lamp::Waiting,
                why.clone(),
                if required { p.fault } else { p.label },
            ),
            Outcome::Skipped(why) => (Lamp::Off, why.clone(), p.label),
        };
        let failed = matches!(check.outcome, Outcome::Fail(_));
        let action = match check.fix.filter(|_| failed) {
            Some(Fix::GetChrome) => p
                .button("get-chrome", "Get Chrome", Tone::Secondary)
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.open_url(GET_CHROME)))
                .into_any_element(),
            Some(Fix::InstallClaude { npm: false }) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(quiet(&p, "npm not found"))
                .child(
                    p.button("install-claude-off", "Install Claude Code", Tone::Secondary)
                        .opacity(0.5),
                )
                .into_any_element(),
            Some(fix) => {
                let label = match fix {
                    Fix::SetUp => "Set up",
                    _ => "Install Claude Code",
                };
                p.button(
                    SharedString::from(format!("fix-{label}")),
                    label,
                    Tone::Secondary,
                )
                .when(self.fixing, |button| button.opacity(0.5))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.fix(vec![fix], cx)))
                .into_any_element()
            }
            None => div().into_any_element(),
        };
        let title = if required {
            check.id.title().to_owned()
        } else {
            format!("{} (optional)", check.id.title())
        };
        row(title, lamp_line(&p, state, text, color), action)
    }

    /// Stands in for the tabs while a required check fails, or when asked for from About.
    fn first_run_screen(&self, cx: &mut Context<Self>) -> Div {
        let p = theme::palette(cx);
        let checks = self.preflight.clone().unwrap_or_else(Preflight::checking);
        let entries = checks
            .checks
            .iter()
            .map(|check| self.check_row(check, cx))
            .collect();
        let required = checks.required_fixes();
        let mut footer = div().flex().justify_end().gap_2().child(
            p.button("check-again", "Check again", Tone::Secondary)
                .when(checks.running(), |button| button.opacity(0.5))
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.recheck(cx))),
        );
        if !required.is_empty() {
            footer = footer.child(
                p.button(
                    "set-up-everything",
                    if self.fixing {
                        "Setting up…"
                    } else {
                        "Set up everything"
                    },
                    Tone::Primary,
                )
                .when(self.fixing, |button| button.opacity(0.5))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.fix(required.clone(), cx);
                })),
            );
        } else if checks.ready() {
            footer = footer.child(p.button("continue", "Continue", Tone::Primary).on_click(
                cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.first_run = false;
                    view.first_run_asked = false;
                    cx.notify();
                }),
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                p.panel()
                    .px(px(20.))
                    .py(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .py(px(8.))
                            .child(heading(&p, "Set up Vexdom"))
                            .child(quiet(
                                &p,
                                "The first two are needed before Vexdom can run; the others help.",
                            )),
                    )
                    .child(rows(&p, entries)),
            )
            .child(footer)
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_label) && self.tab == Tab::Overview && !self.first_run {
            window.focus(&self.label.focus_handle(cx));
        }
        let p = theme::palette(cx);
        let body = if self.first_run {
            self.first_run_screen(cx)
        } else {
            match self.tab {
                Tab::Overview => self.overview(cx),
                Tab::Browsers => self.browsers_tab(cx),
                Tab::Agent => self.agent_tab(cx),
                Tab::Settings => self.settings_tab(cx),
                Tab::About => self.about_tab(cx),
            }
        };
        let strip = if self.first_run {
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.label)
                .child("Vexdom")
        } else {
            let mut tabs = div()
                .flex()
                .gap(px(2.))
                .p(px(2.))
                .rounded(px(7.))
                .bg(p.well_hover);
            for (tab, label) in Tab::ALL {
                tabs = tabs.child(self.tab_button(tab, label, cx));
            }
            tabs
        };
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.ground)
            .font_family(theme::FAMILY)
            .text_color(p.ink)
            .child(
                div()
                    .h(px(TITLE_STRIP))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(strip),
            )
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(22.))
                    .pt(px(8.))
                    .pb(px(16.))
                    .child(body),
            );
        if let Some(line) = &self.line {
            root = root.child(
                div()
                    .flex_none()
                    .px(px(26.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .text_color(if line.error { p.fault } else { p.label })
                    .child(line.text.clone()),
            );
        }
        root
    }
}
