//! `vexdom` commands (`src/daemon/cli.ts`, plus the agent-browser-style page commands of ADR-025).

mod browsers;
mod completions;
mod confirm;
mod curl_cookies;
pub(crate) mod diff;
mod doctor;
mod flows;
mod install;
mod page;
mod plugins;
mod setup;
pub(crate) mod state;
mod uninstall;
mod update;
mod verbs;

pub(crate) use page::prefs as project_prefs;

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use vexdom_core::{
    agents, approvals, config, downloads, lockfile, paths, schedules, skills, tools,
};
use vexdom_pure::schedules as pure_schedules;
use vexdom_schema::Topic;

use vexdom_client::{Bridge, block_on};

pub const VERSION: &str = vexdom_daemon::VERSION;

fn usage() -> String {
    format!(
        "vexdom {VERSION} — Vexdom Bridge, the half of Vexdom that runs on your computer

  vexdom setup            start the Bridge, then add the extension to a browser and pair the two
                              --browser chrome|edge|brave|arc|vivaldi|opera|chromium|firefox skips the question
                              --unpacked writes the folder to load unpacked, in any browser
                              --no-open, --no-wait: print the store link, and do not wait for the browser
  vexdom update           install the latest signed release over this copy (the tarball or Vexdom.app) and restart
                              the Bridge; --check only says whether one is out, --local re-runs setup with this
                              binary, --no-restart, --json
  vexdom doctor           check this install, the Bridge, browsers, agent, config and permissions; --fix repairs
                              what it safely can, --offline skips network checks, --json
  vexdom uninstall        stop the Bridge and remove everything Vexdom wrote
  vexdom browsers         the browsers on this computer, where each gets the extension, and which are connected
  vexdom pair             issue a one-time code to type into the extension
  vexdom pairing [mode]   how browsers pair by themselves: auto, ask (you allow each one) or code (only with a code)
  vexdom requests         browsers waiting for you to allow them, and actions waiting for your approval
  vexdom approve [id]     allow a waiting or unpaired browser, or an action waiting for approval (c-…, --always
                              for that site from now on); --watch asks about each one as it comes
  vexdom deny <id>        turn a waiting browser down, or refuse a waiting action
  vexdom status           the Bridge, each paired browser, and the agent
  vexdom sessions         list paired browsers, and the ones kept out
  vexdom rename <id> <name>   name a paired browser, e.g. \"Work Chrome\"
  vexdom revoke [id]      unpair one browser by the id \"sessions\" prints, or all of them; it stays out until you reconnect it

  vexdom snapshot         the active tab's elements, each with a ref (e5); then only what changed
                              -i interactive only (default), --all every element, -c compact, -d N depth, -s SEL subtree,
                              -u link targets, --full, --json {{text, refs}}
  <t> is a ref (@e5) or a CSS selector; each element command prints the result, then the snapshot delta it caused
  vexdom click <t>        [--button left|right|middle] [--modifiers ctrl,shift] [--new-tab]
  vexdom dblclick <t>     [--modifiers …]
  vexdom fill <t> <text>  replace a field's value
  vexdom type <t> <text>  type key by key; --clear empties it first, --delay MS between keys
  vexdom press|key [<t>] <key>   e.g. Enter, Control+a;  vexdom keydown|keyup <key>
  vexdom keyboard type|inserttext <text>   into the focused element
  vexdom hover|focus|submit|highlight|scrollintoview <t>
  vexdom select <t> <value…>   [--by value|label]; several values for a multiple select
  vexdom check|uncheck <t>
  vexdom scroll up|down|left|right|top|bottom [px] [--selector SEL]
  vexdom drag <from> <to>
  vexdom upload <t> <file…>     vexdom download <t> [path]
  vexdom get text|html|value|attr|count|box|styles|title|url [<t>] [name]
  vexdom is visible|enabled|checked <t>
  vexdom find role|text|label|placeholder|alt|title|testid <value> <command> [arg] [--name N]
  vexdom find first|last <sel> <command> [arg];  vexdom find nth <n> <sel> <command> [arg]
  vexdom wait <ms> | wait <t> [--state S] [--timeout MS] | wait --text|--url|--load|--fn <v> | wait --download [path]
  vexdom open|goto|navigate <url>    vexdom back|forward|reload    vexdom pushstate <url>
  vexdom tab [<id>|<text>] | tab new <url> | tab close [<id>|<text>]    vexdom window new [url]
  vexdom frame <t>|main   enter an iframe, or go back to the top document
  vexdom eval <js>        run JavaScript in the page; --stdin reads it from stdin, -b <base64> decodes it
  vexdom storage local|session [key] | … set <k> <v> | … remove <k> | … clear
  vexdom cookies [get [name]] | cookies set <name> <value> [--url --domain --path --secure --http-only --same-site --expires] | cookies clear [name]
  vexdom clipboard read|write <text>|copy|paste
  vexdom webmcp list [tool] [--frame id] | webmcp invoke <tool> [--params <json|@file>] [--detach] | webmcp result|cancel <id>
  vexdom read [url]       the page as markdown; --outline headings only, --filter TEXT matching sections, --offset N from char N
  vexdom screenshot       capture the tab; -o FILE, --screenshot-dir DIR, --screenshot-format png|jpeg,
                              --screenshot-quality N, --full, -s <t>, --if-changed [--threshold F], --annotate
  vexdom diff snapshot [--baseline FILE]   what changed since the last snapshot, or since a saved one
  vexdom diff screenshot --baseline FILE [-o OUT] [--threshold F]    vexdom diff url <u1> <u2> [--screenshot]
  vexdom state save|load|show <name|path> | state list | state rename <old> <new> | state clear [name|--all]
                          | state clean --older-than DAYS: cookies and web storage, kept in ~/.vexdom/states
  vexdom read <url>       a page over HTTP without a tab; --raw, --require-md, --llms index|full, --timeout MS
  vexdom console [--level L] [--clear] | errors | network requests [--filter s] | network request <id>
  vexdom network route <pattern> [--abort | --body <json|@file>] | network unroute [pattern] | network har start|stop [path]
  vexdom mouse move <x> <y> [--human] | mouse down|up [button] | mouse wheel <dy> [dx]
  vexdom dialog [status] | dialog accept [text] | dialog dismiss | dialog auto on|off
  vexdom set viewport <w> <h> | set device <name> | set geo <lat> <lon> | set offline [on|off] | set headers <json>
                          | set credentials <user> <pass> | set media [screen|print] [--color-scheme s] | set reset
  vexdom pdf [path]       vexdom record start <path> [--fps n] | record stop     vexdom trace|profiler start|stop [path]
  vexdom vitals [--reload]  vexdom react tree|inspect <id>|renders start|stop|suspense     vexdom a11y [--tags csv]
  vexdom initscript add <file|-> | initscript remove <id> | initscript list
  vexdom batch [--bail] [--json] [--omit-command] [\"<command>\" …]   steps as arguments, a JSON array of string
                              arrays on stdin, or one per stdin line: {{\"action\",\"input\"}} or \"click @e3\"
      page commands take --tab <id>, --max-output <n> and --json ({{\"ok\",\"data\"|\"error\"}}); ./vexdom.json sets defaults

  vexdom agent            show which agent runs the side panel, and which are installed
  vexdom agent <name>     switch to claude
  vexdom agent fix <name> let Vexdom fix what that agent still needs
  vexdom agent model <name> [model]   pin that agent's model, or omit it for the CLI's default
  vexdom agent models <name>          list the models that agent offers; --refresh asks its CLI again

  vexdom skills           list the skills the agent can route to, and where they came from
  vexdom flow record start|stop|status   record a task in this tab; vexdom flow steps lists what was captured
  vexdom flow save <name> [--param k=v]… [--from n --to n] [--output id]   keep its requests as a saved flow
  vexdom flow run <name> [--param k=v]… [--allow-write all|ids] [--where f=v]… [--fields a,b] [--limit n]
                          rerun it as direct requests from the page; vexdom flow list [--origin h]
  vexdom auth save <name> --url u --username u --password-stdin   a login, encrypted in ~/.vexdom/vault
  vexdom auth login <name> | auth list | auth show <name> | auth delete <name>
  vexdom plugin [list]    the plugins in ~/.vexdom/config.json (or VEXDOM_PLUGINS), with their capabilities
  vexdom plugin add <npm package|owner/repo|./file> [--name N] [--capability C]… [--no-manifest]
                              register a plugin; its name and capabilities come from its manifest unless given
  vexdom plugin show <name>       vexdom plugin run <name> <type> [--payload <json>]
  vexdom approvals        list the “always on this site” approvals you have granted
  vexdom approvals clear [host]   forget them, all of them or one site's
  vexdom tasks            list the scheduled tasks, when each runs next, and how it last went
  vexdom tasks pause|resume [id]   pause or resume one task, or every task at once
  vexdom tasks delete <id>         delete a task
  vexdom downloads        list the files captured from pages, and where they were saved
  vexdom downloads clear  delete all of them
  vexdom tools            print the bundled tool manifest (no browser needed)
  vexdom logs             print the Bridge's log
  vexdom start            bring the Bridge up in the background, if it is not already
  vexdom stop             stop the Bridge
  vexdom restart          stop the Bridge and bring up a fresh one
  vexdom token            print the control token (for MCP clients, not the browser)
  vexdom completions <shell>   print the completion script for bash, zsh, fish or powershell

  setup, browsers, agent, skills, approvals, tasks and downloads take --json, which is what the apps read.
  vexdom --version        print the version

Getting started:  vexdom setup

For MCP clients
  vexdom mcp              serve MCP over stdio — what a client runs, not what you type
  vexdom mcp call <tool> [json]   call one tool the way an MCP client would
      claude mcp add vexdom -- vexdom mcp
"
    )
}

/// The parsed command line, TS style: positionals skip `--flags` and the values of value flags.
pub struct Args {
    pub all: Vec<String>,
}

const VALUE_FLAGS: &[&str] = &[
    "--name",
    "--tab",
    "--max-output",
    "--offset",
    "-d",
    "-s",
    "-o",
    "--ref",
    "--filter",
    "--target",
    "--browser",
    "--button",
    "--modifiers",
    "--by",
    "--text",
    "--url",
    "--load",
    "--fn",
    "--state",
    "--timeout",
    "--selector",
    "--params",
    "--baseline",
    "--threshold",
    "--screenshot-dir",
    "--screenshot-format",
    "--screenshot-quality",
    "--older-than",
    "--delay",
    "--domain",
    "--path",
    "--same-site",
    "--expires",
    "--output",
    "-b",
    "--base64",
    "--level",
    "--contains",
    "--limit",
    "--type",
    "--method",
    "--status",
    "--body",
    "--content-type",
    "--allow-origin",
    "--resource-type",
    "--content",
    "--seed",
    "--steps",
    "--duration",
    "--scale",
    "--accuracy",
    "--color-scheme",
    "--reduced-motion",
    "--fps",
    "--contact-sheet-threshold",
    "--categories",
    "--tags",
    "--init-script",
    "--frame",
    "--wait",
    "--label",
    "--curl",
    "--param",
    "--origin",
    "--allow-write",
    "--where",
    "--fields",
    "--from",
    "--to",
    "--description",
    "--username",
    "--password",
    "--username-selector",
    "--password-selector",
    "--submit-selector",
    "--credential-provider",
    "--item",
    "--capability",
    "--capabilities",
    "--payload",
    "--llms",
];

impl Args {
    pub fn flag(&self, name: &str) -> bool {
        self.all.iter().any(|arg| arg == name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        let at = self.all.iter().position(|arg| arg == name)?;
        self.all.get(at + 1).map(String::as_str)
    }

    /// Every value of a flag given more than once (`--param a=1 --param b=2`).
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.all
            .windows(2)
            .filter(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
            .collect()
    }

    /// Every word after a lone `--` is positional, verbatim.
    pub fn positional(&self) -> Vec<&str> {
        let mut out = Vec::new();
        let mut skip = false;
        let mut verbatim = false;
        for arg in self.all.iter().skip(1) {
            if verbatim {
                out.push(arg.as_str());
                continue;
            }
            if arg == "--" {
                verbatim = true;
                continue;
            }
            if skip {
                skip = false;
                continue;
            }
            if VALUE_FLAGS.contains(&arg.as_str()) {
                skip = true;
                continue;
            }
            if arg.starts_with("--")
                || (arg.len() == 2
                    && arg.starts_with('-')
                    && !arg[1..].chars().all(|c| c.is_ascii_digit()))
            {
                continue;
            }
            out.push(arg.as_str());
        }
        out
    }

    pub fn json(&self) -> bool {
        self.flag("--json")
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}

pub fn run(argv: Vec<std::ffi::OsString>) -> ExitCode {
    let all: Vec<String> = argv
        .iter()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let args = Args { all };
    let command = args.all.first().map(String::as_str);
    match command {
        None => {
            if std::env::var("VEXDOM_AGENT_RUN").is_ok_and(|run| !run.is_empty()) {
                crate::mcp::run(crate::mcp::Options { max_output: None })
            } else {
                println!("{}", usage());
                ExitCode::SUCCESS
            }
        }
        Some("mcp") if args.all.get(1).is_some_and(|word| word == "call") => {
            match args.all.get(2) {
                Some(tool) => crate::mcp::call(tool, args.all.get(3).map(String::as_str)),
                None => fail("Usage: vexdom mcp call <tool> [json]"),
            }
        }
        Some("mcp") => crate::mcp::run(crate::mcp::Options {
            max_output: args.value("--max-output").and_then(|max| max.parse().ok()),
        }),
        Some("daemon") => vexdom_daemon::run(vexdom_daemon::Options {
            no_idle_exit: args.flag("--no-idle-exit"),
        }),
        Some("setup") => setup::setup(&args.all[1..]),
        Some("update") => update::run(&args),
        Some("doctor") => doctor::run(&args),
        Some("plugin") => plugins::run(&args),
        Some(verb @ ("flow" | "auth")) => flows::run(verb, &args),
        Some("uninstall") => setup::uninstall(&args.all[1..]),
        Some("native-host") => setup::native_host(),
        Some("browsers") => setup::browsers(args.json()),
        Some("pair") => block_on(pair()),
        Some("sessions") => block_on(sessions()),
        Some("revoke") => block_on(revoke(args.all.get(1).map(String::as_str))),
        Some("pairing") => block_on(set_pairing(args.positional().first().copied())),
        Some("requests") => block_on(requests(args.json())),
        Some("approve") => block_on(approve(&args)),
        Some("deny") => block_on(deny(args.positional().first().copied())),
        Some("rename") => {
            let positional = args.positional();
            let name = (positional.len() > 1).then(|| positional[1..].join(" "));
            block_on(rename(positional.first().copied(), name))
        }
        Some("agent") => block_on(agent(&args)),
        Some("tools") => print_tools(),
        Some("status") => block_on(status()),
        Some("start") => block_on(start()),
        Some("stop") => block_on(stop()),
        Some("restart") => block_on(restart()),
        Some("skills") => print_skills(args.json()),
        Some("approvals") => manage_approvals(&args),
        Some("downloads") => manage_downloads(&args),
        Some("tasks") => manage_tasks(&args),
        Some("logs") => show_logs(),
        Some("token") => print_token(),
        Some("completions") => completions::print_script(args.all.get(1).map(String::as_str)),
        Some("__complete") => completions::complete_command(&args.all[1..]),
        Some("--version" | "-v") => {
            println!("{VERSION}");
            ExitCode::SUCCESS
        }
        Some("help" | "--help" | "-h") => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        Some(verb) if page::is_page_command(verb) => page::run(verb, &args),
        Some(other) => {
            eprintln!("Unknown command \"{other}\"\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}

pub async fn connect() -> Result<Bridge, ExitCode> {
    Bridge::connect(None).await.map_err(|error| fail(&error))
}

fn group_code(code: &str) -> String {
    if code.len() > 4 {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    }
}

async fn pair() -> ExitCode {
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let Ok(reply) = bridge.control.pair_request().send().promise.await else {
        return fail("The Vexdom daemon did not respond");
    };
    let Ok(reply) = reply.get() else {
        return fail("The Vexdom daemon did not respond");
    };
    let code = reply
        .get_code()
        .ok()
        .and_then(|code| code.to_string().ok())
        .unwrap_or_default();
    let minutes = ((reply.get_expires_at() - vexdom_core::log::now_ms()) as f64 / 60_000.0).round();
    println!("\n  Pairing code:  {}\n", group_code(&code));
    println!("  Open the Vexdom side panel, paste it, and press Connect.");
    println!("  Expires in {minutes} minutes and works once.\n");
    ExitCode::SUCCESS
}

fn text_of(reader: capnp::Result<capnp::text::Reader<'_>>) -> String {
    vexdom_daemon::wire::text(reader)
}

fn paired_by_text(by: &str) -> &'static str {
    match by {
        "auto" => "paired by itself",
        "approved" => "allowed and paired",
        _ => "paired with a code",
    }
}

async fn sessions() -> ExitCode {
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let Ok(reply) = bridge.control.sessions_request().send().promise.await else {
        return fail("The Vexdom daemon did not respond");
    };
    let list = reply.get().and_then(|reply| reply.get_sessions());
    let Ok(list) = list else {
        return fail("The Vexdom daemon did not respond");
    };
    if list.is_empty() {
        println!("No paired browsers. Run \"vexdom setup\" to add one.");
    }
    for session in list.iter() {
        let label = text_of(session.get_label());
        let browser = text_of(session.get_browser());
        let origin = text_of(session.get_origin());
        let shown = [label, browser]
            .into_iter()
            .find(|text| !text.is_empty())
            .unwrap_or_else(|| origin.clone());
        println!(
            "{} {shown}  {}",
            if session.get_connected() {
                "●"
            } else {
                "○"
            },
            text_of(session.get_id())
        );
        let source = text_of(session.get_source());
        println!(
            "    {} ({origin}), extension v{}, {} {}, last seen {}",
            vexdom_core::pairing::source_label(if source.is_empty() {
                "unpacked"
            } else {
                &source
            }),
            text_of(session.get_extension_version()),
            paired_by_text(&text_of(session.get_paired_by())),
            text_of(session.get_paired_at()),
            text_of(session.get_last_seen_at())
        );
    }
    show_blocked(&bridge).await;
    ExitCode::SUCCESS
}

async fn show_blocked(bridge: &Bridge) {
    let Ok(reply) = bridge.control.blocked_request().send().promise.await else {
        return;
    };
    let blocked: Value = reply
        .get()
        .ok()
        .and_then(|reply| serde_json::from_str(&text_of(reply.get_blocked())).ok())
        .unwrap_or(Value::Null);
    let Some(list) = blocked.as_array().filter(|list| !list.is_empty()) else {
        return;
    };
    println!("\nKept out until you reconnect them:");
    for browser in list {
        let field = |key: &str| browser.get(key).and_then(Value::as_str).unwrap_or("");
        let name = if field("browser").is_empty() {
            field("origin")
        } else {
            field("browser")
        };
        println!(
            "  {name}  {}  {} {}",
            field("installId"),
            field("reason"),
            field("at")
        );
    }
    println!(
        "Let one back in with \"vexdom approve <id>\", then press Reconnect in its Vexdom side panel."
    );
}

async fn revoke(browser: Option<&str>) -> ExitCode {
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let mut request = bridge.control.revoke_request();
    match browser {
        Some(target) if target.contains("://") => request.get().set_origin(target),
        Some(target) => request.get().set_session(target),
        None => {}
    }
    let Ok(reply) = request.send().promise.await else {
        return fail("The Vexdom daemon did not respond");
    };
    let revoked = reply.get().map(|reply| reply.get_revoked()).unwrap_or(0);
    if revoked == 0 {
        println!(
            "{}",
            browser.map_or_else(
                || "Nothing to revoke.".to_owned(),
                |b| format!("No session for {b}.")
            )
        );
        return ExitCode::SUCCESS;
    }
    println!("Revoked {revoked} session(s). They will not pair again by themselves.");
    println!("Reconnect one from its extension, or pair it with a code from \"vexdom pair\".");
    ExitCode::SUCCESS
}

pub fn pairing_line(mode: &str) -> &'static str {
    match mode {
        "ask" => "ask: a browser connects once you allow it, in the app or with \"vexdom approve\"",
        "code" => "code only: a browser connects once you type a code from \"vexdom pair\" into it",
        _ => "automatic: a browser with Vexdom connects by itself",
    }
}

async fn set_pairing(mode: Option<&str>) -> ExitCode {
    if let Some(mode) = mode
        && !vexdom_core::pairing::is_pairing_mode(mode)
    {
        return fail(&format!(
            "Pairing is {}, not \"{mode}\".",
            vexdom_core::pairing::PAIRING_MODES.join(", ")
        ));
    }
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let mut request = bridge.control.pairing_request();
    request.get().set_set(mode.unwrap_or(""));
    let Ok(reply) = request.send().promise.await else {
        return fail("The Vexdom daemon did not respond");
    };
    let Ok(reply) = reply.get() else {
        return fail("The Vexdom daemon did not respond");
    };
    let error = text_of(reply.get_error());
    if !error.is_empty() {
        return fail(&error);
    }
    let mode = text_of(reply.get_mode());
    println!("Pairing: {}.", pairing_line(&mode));
    if mode == "ask" {
        println!(
            "{}",
            if text_of(reply.get_approver()) == "app" {
                "The Vexdom app asks you about each new browser."
            } else {
                "Allow each new browser with \"vexdom approve\"."
            }
        );
    }
    ExitCode::SUCCESS
}

/// `formatWhen`.
pub fn format_when(at: i64) -> String {
    let ago = vexdom_core::log::now_ms() - at;
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    if ago < MINUTE {
        "just now".into()
    } else if ago < HOUR {
        format!("{}m ago", ago / MINUTE)
    } else if ago < DAY {
        format!("{}h ago", ago / HOUR)
    } else if ago < 7 * DAY {
        format!("{}d ago", ago / DAY)
    } else {
        jiff::Timestamp::from_millisecond(at)
            .map(|time| {
                time.to_zoned_datetime(jiff::tz::TimeZone::system())
                    .strftime("%b %-d")
                    .to_string()
            })
            .unwrap_or_default()
    }
}

fn describe_request(request: &Value) -> String {
    let field = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or("");
    let source = match field("source") {
        source @ ("firefox" | "unpacked") => vexdom_core::pairing::source_label(source).to_owned(),
        other => other.to_owned(),
    };
    format!(
        "{} ({source}, extension v{})",
        if field("browser").is_empty() {
            "A browser"
        } else {
            field("browser")
        },
        field("extensionVersion")
    )
}

async fn pair_requests(bridge: &Bridge) -> Vec<Value> {
    let Ok(reply) = bridge.control.pair_requests_request().send().promise.await else {
        return Vec::new();
    };
    reply
        .get()
        .ok()
        .and_then(|reply| serde_json::from_str::<Value>(&text_of(reply.get_requests())).ok())
        .and_then(|requests| requests.as_array().cloned())
        .unwrap_or_default()
}

async fn requests(json: bool) -> ExitCode {
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let requests = pair_requests(&bridge).await;
    let confirmations = bridge.confirmations().await;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "requests": requests, "confirmations": confirmations })
            )
            .unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    if !confirmations.is_empty() {
        println!("Actions waiting for your approval:");
        for confirmation in &confirmations {
            println!("  {}", confirm::describe(confirmation));
        }
        println!(
            "\nAllow one with \"vexdom approve <id>\" (--always for this site from now on), or refuse it with \"vexdom deny <id>\".\n"
        );
    }
    if requests.is_empty() {
        println!("No browser is waiting to be allowed.");
        return ExitCode::SUCCESS;
    }
    for request in &requests {
        println!(
            "  {}  {}, asked {}",
            request.get("id").and_then(Value::as_str).unwrap_or(""),
            describe_request(request),
            format_when(request.get("firstAt").and_then(Value::as_i64).unwrap_or(0))
        );
    }
    println!(
        "\nAllow one with \"vexdom approve <id>\", or turn it down with \"vexdom deny <id>\"."
    );
    ExitCode::SUCCESS
}

async fn decide(
    bridge: &Bridge,
    request: &str,
    allow: bool,
    label: Option<&str>,
) -> (bool, Option<String>) {
    let mut call = bridge.control.decide_request();
    call.get().set_request(request);
    call.get().set_allow(allow);
    call.get().set_label(label.unwrap_or(""));
    match call.send().promise.await {
        Ok(reply) => match reply.get() {
            Ok(reply) => (
                reply.get_ok(),
                Some(text_of(reply.get_error())).filter(|e| !e.is_empty()),
            ),
            Err(_) => (false, None),
        },
        Err(_) => (false, None),
    }
}

async fn approve(args: &Args) -> ExitCode {
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    if args.flag("--watch") {
        return watch_requests(&bridge).await;
    }
    if let Some(id) = args
        .positional()
        .first()
        .filter(|id| confirm::is_confirmation(id))
    {
        if let Some(refusal) = confirm::needs_terminal() {
            return fail(refusal);
        }
        if !bridge.confirm(id, true, args.flag("--always")).await {
            return fail(&format!("Nothing is waiting under {id}."));
        }
        println!("Allowed.");
        return ExitCode::SUCCESS;
    }
    let requests = pair_requests(&bridge).await;
    let id = args.positional().first().map(|id| (*id).to_owned());
    let chosen = id.or_else(|| {
        (requests.len() == 1).then(|| {
            requests[0]
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        })
    });
    let Some(chosen) = chosen else {
        return fail(if requests.is_empty() {
            "No browser is waiting to be allowed."
        } else {
            "Several browsers are waiting. Name one: \"vexdom requests\" lists them."
        });
    };
    let (ok, error) = decide(&bridge, &chosen, true, args.value("--name")).await;
    if !ok {
        return fail(&error.unwrap_or_else(|| format!("Nothing is waiting under {chosen}.")));
    }
    let waiting = requests.iter().any(|request| {
        request.get("id").and_then(Value::as_str) == Some(&chosen)
            || request.get("installId").and_then(Value::as_str) == Some(&chosen)
    });
    println!(
        "{}",
        if waiting {
            "Allowed. It connects within a few seconds; \"vexdom status\" shows it."
        } else {
            "Allowed. It connects once you press Reconnect in its Vexdom side panel."
        }
    );
    ExitCode::SUCCESS
}

async fn watch_requests(bridge: &Bridge) -> ExitCode {
    use std::io::IsTerminal as _;
    if !std::io::stdin().is_terminal() {
        return fail("--watch asks you about each browser, so it needs a terminal.");
    }
    let (changed, mut wake) = tokio::sync::mpsc::unbounded_channel::<()>();
    let also = changed.clone();
    let _handle = bridge
        .watch(
            Topic::PairRequestsChanged,
            Some("terminal"),
            false,
            move |_, _| {
                let _ = changed.send(());
            },
        )
        .await;
    let _confirmations = bridge
        .watch(Topic::ConfirmationsChanged, None, false, move |_, _| {
            let _ = also.send(());
        })
        .await;
    println!(
        "Waiting for browsers that ask to pair and actions that ask for approval. Ctrl-C stops."
    );
    let mut asked: Vec<String> = Vec::new();
    loop {
        for confirmation in bridge.confirmations().await {
            let id = confirmation
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if asked.contains(&id) {
                continue;
            }
            asked.push(id.clone());
            print!(
                "\n  {}\n  Allow it? [y/N/a = always on this site] ",
                confirm::describe(&confirmation)
            );
            let _ = std::io::stdout().flush();
            let answer = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
                line
            })
            .await
            .unwrap_or_default();
            let lower = answer.trim().to_lowercase();
            let always = lower == "a" || lower == "always";
            let allow = always || lower == "y" || lower == "yes";
            let found = bridge.confirm(&id, allow, always).await;
            println!(
                "{}",
                match (found, allow) {
                    (false, _) => "  It had already been answered or had timed out.",
                    (true, true) => "  Allowed.",
                    (true, false) => "  Refused.",
                }
            );
        }
        for request in pair_requests(bridge).await {
            let id = request
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if asked.contains(&id) {
                continue;
            }
            asked.push(id.clone());
            print!(
                "\n  {} wants to connect. Allow it? [Y/n] ",
                describe_request(&request)
            );
            let _ = std::io::stdout().flush();
            let answer = tokio::task::spawn_blocking(|| {
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
                line
            })
            .await
            .unwrap_or_default();
            let lower = answer.trim().to_lowercase();
            let allow = !(lower == "n" || lower == "no");
            let (ok, _) = decide(bridge, &id, allow, None).await;
            println!(
                "{}",
                if !ok {
                    "  It had already been decided or had expired."
                } else if allow {
                    "  Allowed."
                } else {
                    "  Turned down."
                }
            );
        }
        if wake.recv().await.is_none() {
            return ExitCode::SUCCESS;
        }
    }
}

async fn deny(id: Option<&str>) -> ExitCode {
    let Some(id) = id else {
        return fail("Name the browser to turn down: \"vexdom requests\" lists them.");
    };
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    if confirm::is_confirmation(id) {
        if !bridge.confirm(id, false, false).await {
            return fail(&format!("Nothing is waiting under {id}."));
        }
        println!("Refused.");
        return ExitCode::SUCCESS;
    }
    let (ok, error) = decide(&bridge, id, false, None).await;
    if !ok {
        return fail(&error.unwrap_or_else(|| format!("Nothing is waiting under {id}.")));
    }
    println!(
        "Turned down. It stays out until someone presses Reconnect in that browser and you allow it."
    );
    ExitCode::SUCCESS
}

async fn rename(id: Option<&str>, name: Option<String>) -> ExitCode {
    let (Some(id), Some(name)) = (id, name) else {
        return fail("Usage: vexdom rename <id> <name>. \"vexdom sessions\" prints the ids.");
    };
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let mut request = bridge.control.label_request();
    request.get().set_session(id);
    request.get().set_label(&name);
    let ok = match request.send().promise.await {
        Ok(reply) => reply.get().map(|reply| reply.get_ok()).unwrap_or(false),
        Err(_) => false,
    };
    if !ok {
        return fail(&format!("No paired browser has the id {id}."));
    }
    println!(
        "{}",
        if name.trim().is_empty() {
            "Cleared its name.".to_owned()
        } else {
            format!("Named it \"{}\".", name.trim())
        }
    );
    ExitCode::SUCCESS
}

async fn agent_state(bridge: &Bridge, set: &str, grant: &str, models: &str) -> Value {
    let mut request = bridge.control.agent_request();
    request.get().set_set(set);
    request.get().set_grant(grant);
    request.get().set_models(models);
    match request.send().promise.await {
        Ok(reply) => reply
            .get()
            .ok()
            .and_then(|reply| serde_json::from_str(&text_of(reply.get_state())).ok())
            .unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

fn model_source(kind: &str, models: &Value) -> String {
    let ids = models
        .get("ids")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let source = match (
        models.get("from").and_then(Value::as_str),
        models.get("at").and_then(Value::as_i64),
    ) {
        (Some("cli"), Some(at)) => format!(
            "{ids} listed by {}, {}",
            agents::agent(kind).map_or(kind, |agent| agent.bin),
            format_when(at)
        ),
        _ => format!("Vexdom's built-in list of {ids}"),
    };
    match models.get("error").and_then(Value::as_str) {
        Some(error) => format!("{source}. The last read failed: {error}"),
        None => format!("{source}."),
    }
}

async fn agent(args: &Args) -> ExitCode {
    let positional = args.positional();
    let mut first = positional.first().copied();
    let mut second = positional.get(1).copied();
    let third = positional.get(2).copied();
    if first == Some("model") {
        let Some(kind) = second.filter(|kind| agents::is_agent_kind(kind)) else {
            return fail(&format!(
                "Name the agent whose model to set. Pick one of: {}",
                agents::AGENT_KINDS.join(", ")
            ));
        };
        if !config::write_agent_model(kind, third) {
            return fail(&format!(
                "\"{}\" is not a model id: it has to start with a letter or digit and hold no spaces.",
                third.unwrap_or("undefined")
            ));
        }
        first = None;
        second = None;
    }
    if first == Some("models") {
        return list_models(second, args).await;
    }
    let grant = matches!(first, Some("fix" | "setup"));
    let named = if grant { second } else { first };
    if let Some(named) = named
        && !agents::is_agent_kind(named)
    {
        return fail(&format!(
            "Unknown agent \"{named}\". Pick one of: {}",
            agents::AGENT_KINDS.join(", ")
        ));
    }
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let kind = named.unwrap_or("");
    let state = agent_state(
        &bridge,
        if grant { "" } else { kind },
        if grant { kind } else { "" },
        "",
    )
    .await;
    if args.json() {
        let mut out = state.clone();
        out["catalog"] = serde_json::to_value(agents::AGENT_LIST).unwrap_or(Value::Null);
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let active = state
        .get("active")
        .and_then(Value::as_str)
        .unwrap_or(agents::DEFAULT_AGENT);
    for runner in state
        .get("runners")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let runner_kind = runner.get("kind").and_then(Value::as_str).unwrap_or("");
        let ready = runner.get("ready") == Some(&Value::Bool(true));
        let version = runner
            .get("version")
            .and_then(Value::as_str)
            .map(|v| format!(" — {v}"))
            .unwrap_or_default();
        println!(
            "{} {:<12} {}{version}",
            if runner_kind == active { "●" } else { "○" },
            agents::agent_label(runner_kind),
            if ready { "ready" } else { "unavailable" }
        );
        if ready && let Some(models) = runner.get("models") {
            println!("    models: {}", model_source(runner_kind, models));
        }
        if let Some(problem) = runner.get("problem") {
            println!(
                "    {}",
                problem.get("message").and_then(Value::as_str).unwrap_or("")
            );
            if let Some(fix) = problem.get("fix").and_then(Value::as_str) {
                println!("    {fix}");
            }
            if problem.get("grantable") == Some(&Value::Bool(true)) {
                println!("    Fix it with \"vexdom agent fix {runner_kind}\".");
            }
        }
    }
    println!("\nThe side panel runs on {}.", agents::agent_label(active));
    ExitCode::SUCCESS
}

async fn list_models(named: Option<&str>, args: &Args) -> ExitCode {
    let Some(kind) = named.filter(|kind| agents::is_agent_kind(kind)) else {
        return fail(&format!(
            "Name the agent whose models to list. Pick one of: {}",
            agents::AGENT_KINDS.join(", ")
        ));
    };
    let bridge = match connect().await {
        Ok(bridge) => bridge,
        Err(code) => return code,
    };
    let state = agent_state(
        &bridge,
        "",
        "",
        if args.flag("--refresh") { kind } else { "" },
    )
    .await;
    let runner = state
        .get("runners")
        .and_then(Value::as_array)
        .and_then(|runners| {
            runners
                .iter()
                .find(|runner| runner.get("kind").and_then(Value::as_str) == Some(kind))
        })
        .cloned()
        .unwrap_or(Value::Null);
    let models = runner
        .get("models")
        .cloned()
        .unwrap_or_else(|| json!({"ids": agents::agent(kind).map(|agent| agent.models.to_vec()).unwrap_or_default(), "from": "catalog"}));
    let model = runner.get("model").and_then(Value::as_str);
    if args.json() {
        let mut out = json!({"agent": kind, "model": model});
        if let Some(object) = models.as_object() {
            for (key, value) in object {
                out[key] = value.clone();
            }
        }
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    let ids: Vec<&str> = models
        .get("ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for id in &ids {
        println!("{} {id}", if Some(*id) == model { "●" } else { " " });
    }
    if let Some(model) = model.filter(|model| !ids.contains(model)) {
        println!("● {model}  (pinned, not listed)");
    }
    println!("\n{}", model_source(kind, &models));
    ExitCode::SUCCESS
}

fn print_tools() -> ExitCode {
    let listed: Vec<Value> = tools::chromium()
        .tools
        .iter()
        .filter(|tool| {
            !tools::DAEMON_INTERNAL_ACTIONS
                .contains(&tool.get("name").and_then(Value::as_str).unwrap_or(""))
        })
        .map(|tool| {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
            let mut out = tool.clone();
            out["name"] = json!(tools::short_name(name));
            out["action"] = json!(name);
            out
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&listed).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

async fn status() -> ExitCode {
    let wake = setup::wake_line();
    let Some(lock) = vexdom_daemon::ensure::probe_existing().await else {
        println!("bridge:    not running");
        println!("{wake}");
        println!(
            "browsers:  unknown until it runs (\"vexdom start\", or open a browser that can start it)"
        );
        return ExitCode::SUCCESS;
    };
    let bridge = match Bridge::connect_to(lock.port, &lock.token, None).await {
        Ok(bridge) => bridge,
        Err(error) => return fail(&error),
    };
    let Ok(status) = bridge.status().await else {
        return fail("The Vexdom daemon did not respond");
    };
    let agents_state = agent_state(&bridge, "", "", "").await;
    println!(
        "bridge:    running on 127.0.0.1:{} (pid {}, v{})",
        status["port"],
        lock.pid,
        status["daemonVersion"].as_str().unwrap_or("")
    );
    println!("{wake}");
    let active = agents_state
        .get("active")
        .and_then(Value::as_str)
        .unwrap_or(agents::DEFAULT_AGENT);
    let runner = agents_state
        .get("runners")
        .and_then(Value::as_array)
        .and_then(|runners| {
            runners
                .iter()
                .find(|runner| runner.get("kind").and_then(Value::as_str) == Some(active))
        });
    let state = match runner {
        Some(runner) if runner.get("ready") == Some(&Value::Bool(true)) => runner
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("ready")
            .to_owned(),
        Some(runner) => runner
            .get("problem")
            .and_then(|problem| problem.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("unavailable")
            .to_owned(),
        None => "unavailable".into(),
    };
    println!("agent:     {} — {state}", agents::agent_label(active));
    let Ok(reply) = bridge.control.sessions_request().send().promise.await else {
        return fail("The Vexdom daemon did not respond");
    };
    let stamp = setup::unpacked_version();
    match reply.get().and_then(|reply| reply.get_sessions()) {
        Ok(sessions) if !sessions.is_empty() => {
            println!("browsers:");
            for session in sessions.iter() {
                let browser = text_of(session.get_browser());
                let origin = text_of(session.get_origin());
                let version = text_of(session.get_extension_version());
                let source = text_of(session.get_source());
                let stale =
                    source == "unpacked" && stamp.as_ref().is_some_and(|stamp| *stamp != version);
                println!(
                    "  {} {:<16} v{:<8} {:<17} {}{}",
                    if session.get_connected() {
                        "●"
                    } else {
                        "○"
                    },
                    if browser.is_empty() {
                        origin
                    } else {
                        browser.clone()
                    },
                    version,
                    vexdom_core::pairing::source_label(if source.is_empty() {
                        "unpacked"
                    } else {
                        &source
                    }),
                    if session.get_connected() {
                        "connected"
                    } else {
                        "not connected"
                    },
                    if stale {
                        format!(
                            ", ↻ at {} loads v{}",
                            setup::extensions_page(&browser),
                            stamp.clone().unwrap_or_default()
                        )
                    } else {
                        String::new()
                    }
                );
            }
        }
        _ => println!("browsers:  none paired — \"vexdom setup\" adds the extension to one"),
    }
    println!(
        "tools:     {}",
        if status["manifestInSync"] == Value::Bool(true) {
            "in sync".to_owned()
        } else {
            format!(
                "the extension's own list (extension v{}, Bridge v{})",
                status["extensionVersion"].as_str().unwrap_or(""),
                status["daemonVersion"].as_str().unwrap_or("")
            )
        }
    );
    println!(
        "pairing:   {}",
        pairing_line(status["pairing"]["mode"].as_str().unwrap_or("auto"))
    );
    if status["pairingPending"] == Value::Bool(true) {
        println!("pairing:   a code is waiting to be entered");
    }
    ExitCode::SUCCESS
}

async fn start() -> ExitCode {
    match vexdom_daemon::ensure::ensure_daemon().await {
        Ok(lock) => {
            println!(
                "Vexdom Bridge running on 127.0.0.1:{} (pid {}, v{}).",
                lock.port, lock.pid, lock.daemon_version
            );
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

/// `Control.stop` first, so the Bridge closes its links cleanly; then signals.
async fn graceful_stop() -> vexdom_daemon::ensure::StopResult {
    if let Ok(Some(bridge)) = Bridge::existing(None).await {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            bridge.control.stop_request().send().promise,
        )
        .await;
    }
    vexdom_daemon::ensure::stop_daemons(Duration::from_secs(5)).await
}

async fn stop() -> ExitCode {
    let result = graceful_stop().await;
    for daemon in &result.stopped {
        println!(
            "Stopped the Bridge (pid {}) on 127.0.0.1:{}.",
            daemon.pid, daemon.port
        );
    }
    for daemon in &result.stubborn {
        eprintln!(
            "The Bridge (pid {}) on 127.0.0.1:{} would not exit — kill it by hand.",
            daemon.pid, daemon.port
        );
    }
    if result.stopped.is_empty() && result.stubborn.is_empty() {
        println!("The Bridge is not answering; nothing to stop.");
    }
    println!(
        "A paired browser leaves it stopped until \"vexdom start\", or an MCP client, starts it again."
    );
    if result.stubborn.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

async fn restart() -> ExitCode {
    let result = graceful_stop().await;
    if let Some(daemon) = result.stubborn.first() {
        return fail(&format!(
            "The Bridge (pid {}) is still exiting — try again in a moment.",
            daemon.pid
        ));
    }
    start().await
}

fn print_skills(json: bool) -> ExitCode {
    let loaded = skills::load_skills();
    let config = config::read_agent_config();
    let uploaded = downloads::uploaded_skills_dir();
    let own = vexdom_daemon::agent::agent_skills(&config, false);
    if json {
        let listed: Vec<Value> = loaded
            .iter()
            .map(|skill| {
                let mut out = serde_json::to_value(skill).unwrap_or(Value::Null);
                if let Some(object) = out.as_object_mut() {
                    object.shift_remove("body");
                }
                if skill.provenance == "generated" {
                    out["path"] = json!(uploaded.join(&skill.name).to_string_lossy());
                }
                out
            })
            .collect();
        let own: Vec<Value> = own
            .as_array()
            .into_iter()
            .flatten()
            .map(
                |skill| json!({"name": skill.get("name"), "description": skill.get("description")}),
            )
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"skills": listed, "dirs": skills::skill_dir_names(), "agent": config.agent, "agentSkills": own}))
                .unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    if loaded.is_empty() {
        println!(
            "No skills found. Looked in:\n  {}",
            skills::skill_dir_names().join("\n  ")
        );
        return ExitCode::SUCCESS;
    }
    for skill in &loaded {
        let scope = if skill.category == "site-exploration" {
            if skill.domains.is_empty() {
                "no domains — @name only".to_owned()
            } else {
                skill.domains.join(", ")
            }
        } else {
            String::new()
        };
        let category = if skill.provenance == "generated" {
            "mapped".to_owned()
        } else {
            skill.category.clone()
        };
        let tags: Vec<String> = [
            skill.source.clone(),
            category,
            scope,
            if skill.is_default {
                "default".into()
            } else {
                String::new()
            },
        ]
        .into_iter()
        .filter(|tag| !tag.is_empty())
        .collect();
        println!("{}  ({})", skill.name, tags.join(" · "));
        if !skill.description.is_empty() {
            println!("  {}", skill.description);
        }
        if skill.provenance == "generated" {
            println!("  {}/", uploaded.join(&skill.name).display());
        }
    }
    println!(
        "\nRead in order: {} (a later one shadows an earlier one by name)",
        skills::skill_dir_names().join(" → ")
    );
    if let Some(list) = own.as_array().filter(|list| !list.is_empty()) {
        println!(
            "\n{}'s own skills (attachable from the panel's / picker):",
            agents::agent_label(&config.agent)
        );
        for skill in list {
            println!(
                "{}",
                skill.get("name").and_then(Value::as_str).unwrap_or("")
            );
            if let Some(description) = skill
                .get("description")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
            {
                println!("  {description}");
            }
        }
    }
    ExitCode::SUCCESS
}

fn manage_approvals(args: &Args) -> ExitCode {
    let positional = args.positional();
    match positional.first() {
        Some(&"clear") => {
            let host = positional.get(1).copied();
            let dropped = approvals::forget_grants(host);
            let suffix = host.map(|host| format!(" for {host}")).unwrap_or_default();
            if dropped > 0 {
                println!(
                    "Forgot {dropped} approval{}{suffix}.",
                    if dropped == 1 { "" } else { "s" }
                );
            } else {
                println!("No approvals to forget{suffix}.");
            }
            ExitCode::SUCCESS
        }
        Some(sub) => {
            println!(
                "Unknown command \"approvals {sub}\". Use \"approvals\" or \"approvals clear [host]\"."
            );
            ExitCode::FAILURE
        }
        None => {
            let grants = approvals::list_grants();
            if args.json() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "grants": grants })).unwrap_or_default()
                );
                return ExitCode::SUCCESS;
            }
            if grants.is_empty() {
                println!("No standing approvals. Every gated action still asks.");
                return ExitCode::SUCCESS;
            }
            println!(
                "{} standing approval{} — these no longer ask:\n",
                grants.len(),
                if grants.len() == 1 { "" } else { "s" }
            );
            for grant in &grants {
                println!(
                    "  {:<24} on {:<28} since {}",
                    grant.action,
                    grant.host,
                    grant.at.get(..10).unwrap_or(&grant.at)
                );
            }
            println!(
                "\nRemove one site with \"vexdom approvals clear <host>\", or all with \"approvals clear\"."
            );
            ExitCode::SUCCESS
        }
    }
}

fn manage_downloads(args: &Args) -> ExitCode {
    let positional = args.positional();
    match positional.first() {
        Some(&"clear") => {
            let dropped = downloads::clear_downloads();
            if dropped > 0 {
                println!(
                    "Deleted {dropped} captured download{}.",
                    if dropped == 1 { "" } else { "s" }
                );
            } else {
                println!("Nothing captured to delete.");
            }
            ExitCode::SUCCESS
        }
        Some(sub) => {
            println!(
                "Unknown command \"downloads {sub}\". Use \"downloads\" or \"downloads clear\"."
            );
            ExitCode::FAILURE
        }
        None => {
            let stored = downloads::stored_downloads();
            let dir = downloads::download_dir();
            if args.json() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"dir": dir.to_string_lossy(), "downloads": stored})
                    )
                    .unwrap_or_default()
                );
                return ExitCode::SUCCESS;
            }
            if stored.is_empty() {
                println!(
                    "Nothing captured. Files land in {} when an agent uses page.captureDownload.",
                    dir.display()
                );
                return ExitCode::SUCCESS;
            }
            println!(
                "{} captured download{} in {}:\n",
                stored.len(),
                if stored.len() == 1 { "" } else { "s" },
                dir.display()
            );
            for download in &stored {
                println!(
                    "  {:<32} {:<34} {}",
                    download.name,
                    download.notes,
                    download.captured_at.get(..10).unwrap_or("")
                );
            }
            println!("\nDelete them all with \"vexdom downloads clear\".");
            ExitCode::SUCCESS
        }
    }
}

fn no_such_task(id: Option<&str>) -> ExitCode {
    match id {
        Some(id) => {
            println!("No task matches \"{id}\". \"vexdom tasks\" lists them with their ids.")
        }
        None => println!("Say which task — \"vexdom tasks\" lists their ids."),
    }
    ExitCode::FAILURE
}

fn manage_tasks(args: &Args) -> ExitCode {
    let positional = args.positional();
    let now = vexdom_core::log::now_ms();
    let id = positional.get(1).copied();
    let name_of = |task: &Value| {
        task.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    match positional.first() {
        Some(&sub @ ("pause" | "resume")) => {
            let enabled = sub == "resume";
            let Some(id) = id else {
                schedules::update_schedules(|state| schedules::set_paused(state, !enabled, now));
                println!(
                    "{}",
                    if enabled {
                        "Scheduled tasks resumed."
                    } else {
                        "Every scheduled task is paused. \"vexdom tasks resume\" starts them again."
                    }
                );
                return ExitCode::SUCCESS;
            };
            let name = schedules::update_schedules(|state| {
                schedules::find_task(state, id).map(|task| {
                    schedules::set_enabled(task, enabled, now);
                    name_of(task)
                })
            });
            match name {
                Some(name) => {
                    println!("{} “{name}”.", if enabled { "Resumed" } else { "Paused" });
                    ExitCode::SUCCESS
                }
                None => no_such_task(Some(id)),
            }
        }
        Some(&"delete") => {
            let name = id.and_then(|id| {
                schedules::update_schedules(|state| {
                    let found = schedules::find_task(state, id)
                        .map(|task| (task.get("id").cloned(), name_of(task)));
                    if let Some((task_id, _)) = &found {
                        state
                            .tasks
                            .retain(|task| task.get("id") != task_id.as_ref());
                    }
                    found.map(|(_, name)| name)
                })
            });
            match name {
                Some(name) => {
                    println!("Deleted “{name}”.");
                    ExitCode::SUCCESS
                }
                None => no_such_task(id),
            }
        }
        Some(sub) => {
            println!(
                "Unknown command \"tasks {sub}\". Use \"tasks\", \"tasks pause|resume [id]\" or \"tasks delete <id>\"."
            );
            ExitCode::FAILURE
        }
        None => {
            let state = schedules::read_schedules();
            if args.json() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"paused": state.paused, "tasks": state.tasks})
                    )
                    .unwrap_or_default()
                );
                return ExitCode::SUCCESS;
            }
            if state.tasks.is_empty() {
                println!(
                    "No scheduled tasks. Create one from the Schedules tab in the side panel."
                );
                return ExitCode::SUCCESS;
            }
            println!(
                "{} scheduled task{}{}:\n",
                state.tasks.len(),
                if state.tasks.len() == 1 { "" } else { "s" },
                if state.paused { " — all paused" } else { "" }
            );
            let tz = jiff::tz::TimeZone::system();
            for task in &state.tasks {
                let next = task
                    .get("nextRunAt")
                    .and_then(Value::as_f64)
                    .map(|at| at as i64);
                let when = if task.get("enabled") != Some(&Value::Bool(true)) {
                    "paused".to_owned()
                } else {
                    match next {
                        None => "no more runs".into(),
                        Some(at) if at <= now && !state.paused => {
                            format!(
                                "due {}, waiting for a browser",
                                pure_schedules::describe_moment(at, &tz)
                            )
                        }
                        Some(at) => format!("next {}", pure_schedules::describe_moment(at, &tz)),
                    }
                };
                let id: String = task
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(8)
                    .collect();
                let name: String = name_of(task).chars().take(28).collect();
                println!(
                    "  {id}  {name:<28} {:<30} {when}",
                    pure_schedules::describe_rule(task.get("rule").unwrap_or(&Value::Null), &tz)
                );
                if let Some(last) = task
                    .get("runs")
                    .and_then(Value::as_array)
                    .and_then(|runs| runs.first())
                {
                    let detail = last
                        .get("headline")
                        .and_then(Value::as_str)
                        .filter(|h| !h.is_empty())
                        .or_else(|| last.get("reason").and_then(Value::as_str));
                    println!(
                        "            last: {}{}",
                        last.get("outcome").and_then(Value::as_str).unwrap_or(""),
                        detail
                            .map(|detail| format!(" — {detail}"))
                            .unwrap_or_default()
                    );
                }
            }
            println!(
                "\nPause one with \"vexdom tasks pause <id>\", or all of them with \"tasks pause\". An id prefix is enough."
            );
            ExitCode::SUCCESS
        }
    }
}

fn show_logs() -> ExitCode {
    match std::fs::read_to_string(paths::log_path()) {
        Ok(text) => print!("{text}"),
        Err(_) => println!("No log at {} yet.", paths::log_path().display()),
    }
    ExitCode::SUCCESS
}

fn print_token() -> ExitCode {
    match lockfile::read(&paths::lockfile()) {
        Some(lock) => {
            println!("{}", lock.token);
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("No lockfile yet — start the Bridge first (\"vexdom start\").");
            ExitCode::SUCCESS
        }
    }
}
