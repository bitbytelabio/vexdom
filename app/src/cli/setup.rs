//! `setup`, `update`, `uninstall`, `browsers` and the wake-up line of `status`
//! (`cli.ts` setup/uninstall/showBrowsers, ADR-025: the launcher runs this binary).

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use vexdom_core::pairing::source_label;
use vexdom_core::{agents, config, lockfile, paths};
use vexdom_daemon::native_host::{self, HostInstall};

use super::browsers::{self, BrowserRow, Located, Session};
use super::install::{self, InstallResult};
use super::uninstall;
use super::{VERSION, group_code};
use vexdom_client::{Bridge, block_on};

const WAIT: Duration = Duration::from_secs(5 * 60);
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Offered when no browser is found, since detection can miss one installed somewhere unusual.
const USUAL_BROWSERS: [&str; 4] = ["chrome", "edge", "brave", "firefox"];
const AGENTS_GUIDE: &str = "https://github.com/bitbytelabio/vexdom/tree/main/docs/guide/agents.md";

fn folder_picker() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("In the folder picker press ⇧⌘G and paste that path.")
    } else if cfg!(target_os = "linux") {
        Some("In the folder picker press Ctrl+L and paste that path.")
    } else {
        None
    }
}

fn configured_dir() -> PathBuf {
    let config = config::read_agent_config();
    paths::extension_dir(config.extension_dir().map(PathBuf::from).as_deref())
}

/// The `wake-up:` line of `vexdom status`.
pub fn wake_line() -> String {
    let waking = native_host::registered_browsers();
    if waking.is_empty() {
        return "wake-up:   not registered — run \"vexdom setup\"".into();
    }
    if lockfile::wake_held() {
        return format!(
            "wake-up:   held since \"vexdom stop\" — \"vexdom start\" lets {} start it again",
            waking.join(", ")
        );
    }
    let outdated = if native_host::registration_current() {
        ""
    } else {
        " (an older Bridge registered it: run \"vexdom setup\")"
    };
    format!(
        "wake-up:   {} can start the Bridge{outdated}",
        waking.join(", ")
    )
}

/// The version `setup` last wrote to the unpacked extension folder.
pub fn unpacked_version() -> Option<String> {
    install::read_stamp(&configured_dir()).map(|stamp| stamp.version)
}

/// Where a browser lists its extensions, from the brand its extension reported.
pub fn extensions_page(brand: &str) -> &'static str {
    browsers::entry(browsers::browser_of(brand).unwrap_or("chrome")).extensions_page
}

pub fn native_host() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(2).collect();
    crate::native_host::run(&argv)
}

pub fn browsers(json: bool) -> ExitCode {
    block_on(async {
        let lock = vexdom_daemon::ensure::probe_existing().await;
        let mut sessions = Vec::new();
        if let Some(lock) = &lock
            && let Ok(bridge) = Bridge::connect_to(lock.port, &lock.token, None).await
        {
            sessions = browsers::sessions(&bridge).await.unwrap_or_default();
        }
        let rows = browsers::browser_rows(&sessions, &browsers::locate_browsers(), VERSION);
        if json {
            let dir = configured_dir();
            let version = install::read_stamp(&dir).map(|stamp| stamp.version);
            let out = browsers::browsers_json(lock.is_some(), &rows, &dir, version);
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            return ExitCode::SUCCESS;
        }
        let shown: Vec<&BrowserRow> = rows
            .iter()
            .filter(|row| row.installed || !row.sessions.is_empty())
            .collect();
        if shown.is_empty() {
            println!(
                "No supported browser found. Chrome, Edge, Brave, Arc, Vivaldi, Opera and Firefox all work."
            );
        }
        for row in shown {
            let state = if row.connected {
                "connected"
            } else if !row.sessions.is_empty() {
                "paired, not connected"
            } else {
                "not added"
            };
            println!(
                "{} {:<9} {:<17} {state}",
                if row.connected { "●" } else { "○" },
                row.label,
                row.store
            );
        }
        if lock.is_none() {
            println!("\nThe Bridge is not running, so which are connected is unknown.");
        }
        println!("\nAdd the extension to one with \"vexdom setup --browser <name>\".");
        ExitCode::SUCCESS
    })
}

fn wake_setup_line(wake: &HostInstall) -> String {
    if let Some(skipped) = &wake.skipped {
        return format!("· Wake-up            not registered: {skipped}");
    }
    if wake.browsers.is_empty() {
        "· Wake-up            no supported browser found, so start the Bridge yourself after a reboot".into()
    } else {
        format!(
            "✓ Wake-up            {} can start the Bridge when it is down",
            wake.browsers.join(", ")
        )
    }
}

fn agent_lines(state: &Value) -> Vec<String> {
    let active = state
        .get("active")
        .and_then(Value::as_str)
        .unwrap_or(agents::DEFAULT_AGENT);
    let ready: Vec<&str> = state
        .get("runners")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|runner| runner.get("ready") == Some(&Value::Bool(true)))
        .filter_map(|runner| runner.get("kind").and_then(Value::as_str))
        .collect();
    let label = agents::agent_label(active);
    if ready.contains(&active) {
        let others: Vec<String> = ready
            .iter()
            .filter(|kind| **kind != active)
            .map(|kind| agents::agent_label(kind))
            .collect();
        let also = if others.is_empty() {
            String::new()
        } else {
            format!(" · also ready: {}", others.join(", "))
        };
        return vec![format!("✓ Agent              {label}{also}")];
    }
    if let Some(first) = ready.first() {
        return vec![
            format!(
                "· Agent              the side panel is set to {label}, which is not ready. {} is:",
                agents::agent_label(first)
            ),
            format!("                     switch with \"vexdom agent {first}\""),
        ];
    }
    let install = agents::agent("claude").map_or("", |agent| agent.install);
    vec![
        "· Agent              none ready yet. The side panel runs an agent CLI you are signed in to:".into(),
        format!("                       {install:<38} then run \"claude\" once to sign in"),
        format!("                     Every agent it works with: {AGENTS_GUIDE}"),
    ]
}

/// What setup was asked to put where. `browser: None` is "another browser", always unpacked.
struct Choice {
    browser: Option<&'static str>,
    unpacked: bool,
}

fn prompt(question: &str) -> String {
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_owned()
}

fn ask(rows: &[BrowserRow]) -> Choice {
    let offered: Vec<&BrowserRow> = rows
        .iter()
        .filter(|row| row.installed || !row.sessions.is_empty())
        .collect();
    let listed: Vec<&BrowserRow> = if offered.is_empty() {
        rows.iter()
            .filter(|row| USUAL_BROWSERS.contains(&row.id))
            .collect()
    } else {
        offered
    };
    println!("  Which browser should get the extension?\n");
    for (index, row) in listed.iter().enumerate() {
        let where_ = if row.source == "firefox" {
            "signed add-on"
        } else {
            "unpacked"
        };
        let state = if row.connected {
            "connected"
        } else if row.installed {
            ""
        } else {
            "not found"
        };
        println!(
            "{}",
            format!(
                "    {:>2}  {:<9} {where_:<17} {state}",
                index + 1,
                row.label
            )
            .trim_end()
        );
    }
    let another = listed.len() + 1;
    println!("    {another:>2}  Another browser: load it unpacked\n");
    let choice = loop {
        let answer = prompt("  › ").to_lowercase();
        let picked: usize = if answer.is_empty() {
            1
        } else {
            answer.parse().unwrap_or(0)
        };
        if picked == another || answer == "unpacked" {
            break Choice {
                browser: None,
                unpacked: true,
            };
        }
        let row = picked
            .checked_sub(1)
            .and_then(|at| listed.get(at).copied())
            .or_else(|| rows.iter().find(|row| row.id == answer));
        if let Some(row) = row {
            break Choice {
                browser: Some(row.id),
                unpacked: row.source == "unpacked",
            };
        }
        println!("  Type a number from 1 to {another}.");
    };
    println!();
    choice
}

fn write_extension(dir: &std::path::Path, force: bool) -> Result<InstallResult, ExitCode> {
    install::install(dir, force).map_err(|error| {
        eprintln!("\n  {}", error.message);
        if let Some(hint) = error.hint {
            eprintln!("  {hint}");
        }
        eprintln!();
        ExitCode::FAILURE
    })
}

/// How the browser connects once the extension is in it, in this Bridge's pairing mode.
fn code_lines(mode: &str, code: Option<&str>) -> Vec<String> {
    let enter = code.map_or_else(
        || "enter a code from \"vexdom pair\".".to_owned(),
        |code| format!("enter this code:  {}", group_code(code)),
    );
    let lead = match mode {
        "auto" => format!("It connects by itself; if it asks for a code, {enter}"),
        "ask" => format!("Allow it here when it asks; or {enter}"),
        _ => enter,
    };
    if code.is_some() {
        vec![
            lead,
            "The code works once and expires in 10 minutes. Need another? \"vexdom pair\"".into(),
        ]
    } else {
        vec![lead]
    }
}

/// What to say when the release has no signed Firefox add-on (no AMO keys, or Mozilla is still
/// signing): no link to a file that answers 404.
fn print_no_signed_addon() {
    println!("  Vexdom {VERSION} has no signed Firefox add-on yet. Newer releases may have one:");
    println!("    {}\n", browsers::releases_page());
}

fn print_store_steps(row: &BrowserRow, opened: bool, mode: &str, code: Option<&str>) {
    let Some(url) = row.store_url.as_deref() else {
        print_no_signed_addon();
        return;
    };
    println!(
        "{}",
        if opened {
            format!("  Opening the {} in {}:", row.store, row.label)
        } else {
            format!("  Open this page in {}:", row.label)
        }
    );
    println!("    {url}\n");
    if row
        .sessions
        .iter()
        .any(|session| session.connected && session.source == "unpacked")
    {
        println!(
            "  {} already runs an unpacked copy. Remove it at {} first, or both will answer.\n",
            row.label, row.extensions_page
        );
    }
    let lines = code_lines(mode, code);
    let mut steps: Vec<String> = row.steps.iter().map(|step| (*step).to_owned()).collect();
    steps.push(format!(
        "Click Vexdom in the toolbar (the puzzle piece lists it; pin it there). {}",
        lines[0]
    ));
    for (index, step) in steps.iter().enumerate() {
        println!("    {}. {step}", index + 1);
    }
    for line in &lines[1..] {
        println!("       {line}");
    }
    println!();
}

fn print_unpacked_steps(
    dir: &std::path::Path,
    row: Option<&BrowserRow>,
    mode: &str,
    code: Option<&str>,
) {
    let lines = code_lines(mode, code);
    println!(
        "  Load it unpacked in {}:\n",
        row.map_or("your browser", |row| row.label)
    );
    println!(
        "    1. Open {} and turn on Developer mode.",
        row.map_or("its extensions page (chrome://extensions)", |row| row
            .extensions_page)
    );
    println!("    2. Press “Load unpacked” and choose:\n");
    println!("         {}\n", dir.display());
    // Browsers refuse their own pages given on the command line, so there is no opening this for
    // them. The folder picker shortcut is the next best thing, and it is where people stall.
    if let Some(picker) = folder_picker() {
        println!("       {picker}");
    }
    println!("    3. Click Vexdom in the toolbar. {}", lines[0]);
    for line in &lines[1..] {
        println!("       {line}");
    }
    println!();
}

/// Nobody to ask. Someone already set up hears what an update leaves them to do; anyone else,
/// where each browser gets the extension, with the choosing left to them.
fn print_plan(
    rows: &[BrowserRow],
    sessions: &[Session],
    mode: &str,
    folder_version: Option<&str>,
    code: Option<&str>,
) {
    if !sessions.is_empty() {
        let mut pages: Vec<&str> = sessions
            .iter()
            .filter(|session| {
                session.source == "unpacked"
                    && folder_version.is_some_and(|version| session.extension_version != version)
            })
            .map(|session| extensions_page(&session.browser))
            .collect();
        pages.dedup();
        if sessions.iter().any(|session| session.source != "unpacked") {
            println!("  Store copies update themselves.");
        }
        for page in pages {
            println!("  Press ↻ on the unpacked Vexdom card at {page} to load this build.");
        }
        println!("  Adding another browser? \"vexdom setup --browser <name>\"\n");
        return;
    }
    println!("  Next, add the extension to your browser:\n");
    let unpacked: Vec<&str> = rows
        .iter()
        .filter(|row| row.source == "unpacked")
        .map(|row| row.label)
        .collect();
    let firefox = rows
        .iter()
        .find(|row| row.id == "firefox")
        .and_then(|row| row.store_url.clone());
    println!("    {}", unpacked.join(", "));
    println!("      load it unpacked: \"vexdom setup --unpacked\" writes the folder and says how");
    println!("    Firefox");
    match firefox {
        Some(url) => println!("      {url}\n"),
        None => println!(
            "      no signed add-on in {VERSION} yet; newer releases: {}\n",
            browsers::releases_page()
        ),
    }
    let lines = code_lines(mode, code);
    println!("  Then click Vexdom in the toolbar. {}", lines[0]);
    for line in &lines[1..] {
        println!("  {line}");
    }
    println!("  On a terminal, \"vexdom setup\" asks which browser and walks you through it.\n");
}

async fn wait_for_browser(
    bridge: &Bridge,
    since: &str,
    label: &str,
    mode: &str,
) -> Option<Session> {
    let line =
        format!("Waiting for {label} to connect…  (Ctrl-C stops waiting; nothing is undone)");
    let spin = std::io::stdout().is_terminal();
    if !spin {
        println!("  {line}");
    }
    let deadline = std::time::Instant::now() + WAIT;
    let mut asked: Vec<String> = Vec::new();
    let mut tick = 0_usize;
    while std::time::Instant::now() < deadline {
        if tick.is_multiple_of(8) {
            let sessions = browsers::sessions(bridge).await.unwrap_or_default();
            if let Some(joined) = sessions
                .into_iter()
                .find(|session| session.connected && session.paired_at.as_str() >= since)
            {
                if spin {
                    print!("\r\x1b[2K");
                }
                return Some(joined);
            }
            if mode == "ask" && std::io::stdin().is_terminal() {
                let requests = super::pair_requests(bridge).await;
                for request in requests {
                    let id = request
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    if asked.contains(&id) {
                        continue;
                    }
                    asked.push(id.clone());
                    if spin {
                        print!("\r\x1b[2K");
                    }
                    let answer = prompt(&format!(
                        "  {} wants to connect. Allow it? [Y/n] ",
                        super::describe_request(&request)
                    ))
                    .to_lowercase();
                    super::decide(bridge, &id, !(answer == "n" || answer == "no"), None).await;
                }
            }
        }
        if spin {
            print!("\r  {} {line}", SPINNER[tick % SPINNER.len()]);
            let _ = std::io::stdout().flush();
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        tick += 1;
    }
    if spin {
        print!("\r\x1b[2K");
    }
    None
}

/// Brings Vexdom Bridge up, then gets the extension into a browser and pairs the two. On a terminal
/// it asks which browser and waits for it to connect; told the browser, it skips the question; with
/// neither it prints the plan, so a script or an app never sits on a question nobody can answer.
pub fn setup(argv: &[String]) -> ExitCode {
    let flag = |name: &str| argv.iter().any(|arg| *arg == format!("--{name}"));
    let value_of = |name: &str| {
        let at = argv.iter().position(|arg| *arg == format!("--{name}"))?;
        argv.get(at + 1).cloned()
    };
    let named = value_of("browser");
    let named: Option<&'static str> = match named {
        None => None,
        Some(id) => match browsers::BROWSER_IDS.iter().find(|known| **known == id) {
            Some(known) => Some(known),
            None => {
                eprintln!(
                    "Unknown browser \"{id}\". Pick one of: {}",
                    browsers::BROWSER_IDS.join(", ")
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let json = flag("json");
    let interactive = !json
        && named.is_none()
        && !flag("unpacked")
        && !flag("no-pair")
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();
    // An explicit --dir is remembered, so `update` lands in the same place rather than laying down a
    // second copy at the default path and leaving the browser pointed at the first.
    let chosen_dir = value_of("dir");
    if let Some(dir) = &chosen_dir {
        config::remember_extension_dir(Some(dir));
    }
    let dir = chosen_dir.map_or_else(configured_dir, |dir| {
        paths::extension_dir(Some(&PathBuf::from(dir)))
    });

    block_on(async move {
        // Restart first, then read the lockfile: a Bridge that keeps running holds the previous
        // build's action registry in memory.
        if flag("restart") {
            let stopped = super::graceful_stop().await;
            if let Some(daemon) = stopped.stubborn.first() {
                eprintln!(
                    "The Bridge (pid {}) is still exiting — try again in a moment.",
                    daemon.pid
                );
                return ExitCode::FAILURE;
            }
        }
        let lock = match vexdom_daemon::ensure::ensure_daemon().await {
            Ok(lock) => lock,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let exe = std::env::current_exe().unwrap_or_default();
        let wake = tokio::task::spawn_blocking(move || native_host::install_native_host(&exe))
            .await
            .unwrap_or(HostInstall {
                launcher: PathBuf::new(),
                browsers: Vec::new(),
                skipped: None,
            });
        let bridge = match Bridge::connect_to(lock.port, &lock.token, None).await {
            Ok(bridge) => bridge,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        let since = vexdom_core::log::iso_now();
        let located: Vec<Located> = browsers::locate_browsers();
        let sessions = browsers::sessions(&bridge).await.unwrap_or_default();
        let mut rows = browsers::browser_rows(&sessions, &located, VERSION);
        // Mozilla signs after the release exists, and a release built without AMO keys has no
        // add-on at all: drop the link rather than print or open a 404. Unreachable keeps it.
        if browsers::signed_addon_attached(VERSION).await == Some(false) {
            for row in rows.iter_mut().filter(|row| row.id == "firefox") {
                row.store_url = None;
            }
        }
        let mode = pairing_mode(&bridge).await;

        if !json {
            println!("\n  Vexdom {VERSION} — your real browser, driven by agents\n");
            println!(
                "  ✓ Vexdom Bridge  running on 127.0.0.1:{} (pid {})",
                lock.port, lock.pid
            );
            println!("  {}", wake_setup_line(&wake));
            for line in agent_lines(&super::agent_state(&bridge, "", "", "").await) {
                println!("  {line}");
            }
            println!();
        }

        let choice = if flag("unpacked") {
            Some(Choice {
                browser: named,
                unpacked: true,
            })
        } else if let Some(id) = named {
            Some(Choice {
                browser: Some(id),
                unpacked: browsers::source_for(id) == "unpacked",
            })
        } else if interactive {
            Some(ask(&rows))
        } else {
            None
        };

        // Written for whoever loads it unpacked, and kept current for whoever already does.
        let folder = if choice.as_ref().is_some_and(|choice| choice.unpacked)
            || install::read_stamp(&dir).is_some()
        {
            match write_extension(&dir, flag("force")) {
                Ok(folder) => Some(folder),
                Err(code) => return code,
            }
        } else {
            None
        };

        let row = choice
            .as_ref()
            .and_then(|choice| choice.browser)
            .and_then(|id| rows.iter().find(|row| row.id == id));
        let unpacked = choice.as_ref().is_some_and(|choice| choice.unpacked);
        let settled = row.and_then(|row| {
            row.sessions
                .iter()
                .find(|session| session.connected && (session.source == "unpacked") == unpacked)
        });
        let code = if flag("no-pair") || settled.is_some() {
            None
        } else {
            pair_code(&bridge).await
        };
        let found = row.and_then(|row| located.iter().find(|candidate| candidate.id == row.id));
        let opened =
            row.is_some() && found.is_some() && !unpacked && settled.is_none() && !flag("no-open");
        if opened
            && let (Some(row), Some(found)) = (row, found)
            && let Some(url) = &row.store_url
        {
            browsers::open_page(url, Some(found));
        }

        if json {
            let out = json!({
                "version": VERSION,
                "daemon": {"port": lock.port, "pid": lock.pid},
                "nativeHost": {"launcher": wake.launcher.to_string_lossy(), "browsers": wake.browsers, "skipped": wake.skipped},
                "pairing": {"mode": mode},
                "extensionDir": folder.as_ref().map(|_| dir.to_string_lossy().into_owned()),
                "unpacked": folder.as_ref().map(|folder| json!({"version": folder.version, "changed": folder.changed, "alreadyCurrent": folder.already_current})),
                "alreadyPaired": !sessions.is_empty(),
                "chosen": row.map(|row| json!({"id": row.id, "unpacked": unpacked, "opened": opened, "connected": settled.is_some()})),
                "pairingCode": code.as_ref().map(|(code, _)| code),
                "expiresAt": code.as_ref().map(|(_, at)| at),
                "browsers": rows,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            return ExitCode::SUCCESS;
        }

        if let Some(folder) = &folder {
            let state = if folder.already_current {
                "already current".to_owned()
            } else {
                format!("{} file(s) written", folder.changed)
            };
            println!("  ✓ Unpacked copy      {}, {state}\n", dir.display());
        }
        let code_text = code.as_ref().map(|(code, _)| code.as_str());
        let Some(_) = &choice else {
            print_plan(
                &rows,
                &sessions,
                &mode,
                folder.as_ref().map(|folder| folder.version.as_str()),
                code_text,
            );
            return ExitCode::SUCCESS;
        };
        if let (Some(row), Some(settled)) = (row, settled) {
            println!(
                "  {} is already connected: Vexdom {} from the {}.",
                row.label,
                settled.extension_version,
                source_label(&settled.source)
            );
            if unpacked {
                println!(
                    "  Press ↻ on its card at {} to load this build.\n",
                    row.extensions_page
                );
            } else {
                println!("  Nothing to do.\n");
            }
            return ExitCode::SUCCESS;
        }
        if unpacked {
            print_unpacked_steps(&dir, row, &mode, code_text);
        } else if let Some(row) = row {
            print_store_steps(row, opened, &mode, code_text);
        }
        let Some((_, expires_at)) = code else {
            return ExitCode::SUCCESS;
        };
        if flag("no-wait") {
            return ExitCode::SUCCESS;
        }
        let label = row.map_or("your browser", |row| row.label);
        match wait_for_browser(&bridge, &since, label, &mode).await {
            Some(joined) => {
                let panel = if browsers::browser_of(&joined.browser) == Some("firefox") {
                    "sidebar"
                } else {
                    "side panel"
                };
                let name = if joined.browser.is_empty() {
                    "Your browser"
                } else {
                    joined.browser.as_str()
                };
                println!(
                    "  ✓ {name} is connected: Vexdom {} from the {}.",
                    joined.extension_version,
                    source_label(&joined.source)
                );
                println!("    Open the {panel} from the toolbar and tell it what to do.\n");
            }
            None => {
                let until = jiff::Timestamp::from_millisecond(expires_at)
                    .map(|at| {
                        at.to_zoned_datetime(jiff::tz::TimeZone::system())
                            .strftime("%H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                println!(
                    "  · Not connected yet. Once Vexdom is in {label}, click it in the toolbar and"
                );
                println!(
                    "    enter the code (it works until {until}), or get a new one with \"vexdom pair\".\n"
                );
            }
        }
        ExitCode::SUCCESS
    })
}

async fn pairing_mode(bridge: &Bridge) -> String {
    let mut request = bridge.control.pairing_request();
    request.get().set_set("");
    match request.send().promise.await {
        Ok(reply) => reply
            .get()
            .map(|reply| vexdom_daemon::wire::text(reply.get_mode()))
            .ok()
            .filter(|mode| !mode.is_empty())
            .unwrap_or_else(|| "code".into()),
        Err(_) => "code".into(),
    }
}

async fn pair_code(bridge: &Bridge) -> Option<(String, i64)> {
    let reply = bridge.control.pair_request().send().promise.await.ok()?;
    let reply = reply.get().ok()?;
    Some((
        vexdom_daemon::wire::text(reply.get_code()),
        reply.get_expires_at(),
    ))
}

/// Remove Vexdom in one command, including a Bridge whose lockfile was deleted before it was stopped.
pub fn uninstall(argv: &[String]) -> ExitCode {
    let flag = |name: &str| argv.iter().any(|arg| *arg == format!("--{name}"));
    let plan = uninstall::plan_uninstall(flag("keep-skills"));
    block_on(async move {
        let daemons = vexdom_daemon::ensure::running_daemons().await;
        println!("\n  Vexdom {VERSION} — uninstall\n");
        if daemons.is_empty() && plan.removals.is_empty() {
            println!("  Nothing to remove; this machine is already clean.\n");
            return ExitCode::SUCCESS;
        }
        println!("  This removes:\n");
        for daemon in &daemons {
            println!(
                "    Bridge      127.0.0.1:{}, pid {}",
                daemon.port, daemon.pid
            );
        }
        let waking = native_host::registered_browsers();
        if !waking.is_empty() {
            println!(
                "    wake-up     the native host registered with {}",
                waking.join(", ")
            );
        }
        for removal in &plan.removals {
            println!("    {:<11} {}", removal.label, removal.path.display());
            println!(
                "                {}{}",
                removal.holds,
                if removal.keep.is_empty() {
                    ""
                } else {
                    " — keeping skills/"
                }
            );
        }
        if !plan.elsewhere.is_empty() {
            println!("\n  Left alone — configuration put these outside the two roots:\n");
            for dir in &plan.elsewhere {
                println!(
                    "    {:<11} {}  ({})",
                    dir.label,
                    dir.path.display(),
                    dir.holds
                );
            }
        }
        println!("\n  You will have to remove these yourself:\n");
        println!("    Vexdom in each browser: right-click its toolbar icon and choose Remove.");
        println!("    That is also what clears recordings and held secrets, which live in the");
        println!("    extension’s storage rather than on disk. Remove an unpacked copy first:");
        println!("    take its folder away while it is loaded and the browser holds a broken one.");
        println!("\n    the entry in your MCP client, e.g.  claude mcp remove vexdom");

        if flag("dry-run") {
            println!("\n  --dry-run: nothing was removed.\n");
            return ExitCode::SUCCESS;
        }
        if !flag("yes") && !argv.iter().any(|arg| arg == "-y") {
            if !std::io::stdin().is_terminal() {
                eprintln!(
                    "\n  Nothing was removed — this is not a terminal, so there is nobody to ask."
                );
                eprintln!("  Re-run it with --yes.\n");
                return ExitCode::FAILURE;
            }
            let answer = prompt("\n  Remove all of it? [y/N] ").to_lowercase();
            if !(answer == "y" || answer == "yes") {
                println!("\n  Nothing was removed.\n");
                return ExitCode::SUCCESS;
            }
        }
        println!();

        // Ask the Bridge to drop its session keys before stopping it, so a browser still holding the
        // socket is told it is unpaired. Never start one to do this.
        if let Some(lock) = vexdom_daemon::ensure::probe_existing().await {
            let revoked = match Bridge::connect_to(lock.port, &lock.token, None).await {
                Ok(bridge) => match bridge.control.revoke_request().send().promise.await {
                    Ok(reply) => reply.get().map(|reply| reply.get_revoked()).ok(),
                    Err(_) => None,
                },
                Err(_) => None,
            };
            match revoked {
                Some(0) => {}
                Some(count) => println!(
                    "  ✓ Unpaired   {count} browser{}",
                    if count == 1 { "" } else { "s" }
                ),
                None => println!("  · Unpair     skipped, the Bridge did not answer"),
            }
        }
        // Before the Bridge goes, or a browser that sees it go could have the host start another.
        let unregistered = native_host::remove_native_host();
        if !unregistered.is_empty() {
            println!(
                "  ✓ Wake-up    unregistered from {} place{}",
                unregistered.len(),
                if unregistered.len() == 1 { "" } else { "s" }
            );
        }
        let stopped = vexdom_daemon::ensure::stop_daemons(Duration::from_secs(5)).await;
        if !stopped.stopped.is_empty() {
            let pids: Vec<String> = stopped
                .stopped
                .iter()
                .map(|daemon| daemon.pid.to_string())
                .collect();
            println!("  ✓ Bridge     stopped (pid {})", pids.join(", "));
        }
        for daemon in &stopped.stubborn {
            println!(
                "  ✗ Bridge     pid {} would not exit — kill it by hand",
                daemon.pid
            );
        }
        for outcome in uninstall::remove_all(plan.removals) {
            if !outcome.removed {
                println!(
                    "  ✗ {} — {}",
                    outcome.removal.path.display(),
                    outcome.error.unwrap_or_default()
                );
            } else if !outcome.kept.is_empty() {
                let kept: Vec<String> = outcome
                    .kept
                    .iter()
                    .map(|entry| format!("{entry}/"))
                    .collect();
                println!(
                    "  ✓ Emptied    {} — kept {}",
                    outcome.removal.path.display(),
                    kept.join(", ")
                );
            } else {
                println!("  ✓ Removed    {}", outcome.removal.path.display());
            }
        }
        println!("\n  Done. Remove Vexdom from your browsers if you have not.\n");
        if stopped.stubborn.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    })
}
