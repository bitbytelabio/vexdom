//! Shell completions. `vexdom completions <shell>` prints a small script; on Tab it runs
//! `vexdom __complete <cword> <words…>`, and this module answers from one command tree, with the
//! names it can read locally (saved states, plugins, scheduled tasks, agents, models, MCP tools,
//! approval hosts). It never connects to the Bridge.

use std::io::Write as _;
use std::process::ExitCode;

use vexdom_core::agents::AGENT_LIST;

use self::Value::{Choices, Dir, File, Free, Names};
use super::fail;

/// Names read from local files when completing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    States,
    Plugins,
    Tasks,
    Agents,
    Models,
    McpTools,
    ApprovalHosts,
}

/// What a flag value or a positional is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Free,
    File,
    Dir,
    Choices(&'static [&'static str]),
    Names(Source),
}

struct Flag {
    name: &'static str,
    about: &'static str,
    /// `None`: a boolean flag.
    value: Option<Value>,
}

struct Node {
    name: &'static str,
    about: &'static str,
    /// A top-level page command: also takes `PAGE_FLAGS`.
    page: bool,
    flags: &'static [Flag],
    subs: &'static [Node],
    /// Positional `i`, after the last sub matched, completes `args[i]`.
    args: &'static [Value],
    /// Positionals past `args.len()`; `None` completes nothing.
    more: Option<Value>,
}

const fn node(name: &'static str, about: &'static str) -> Node {
    Node {
        name,
        about,
        page: false,
        flags: &[],
        subs: &[],
        args: &[],
        more: None,
    }
}

/// A top-level page command.
const fn page(name: &'static str, about: &'static str) -> Node {
    Node {
        page: true,
        ..node(name, about)
    }
}

const fn flag(name: &'static str, about: &'static str) -> Flag {
    Flag {
        name,
        about,
        value: None,
    }
}

const fn opt(name: &'static str, about: &'static str, value: Value) -> Flag {
    Flag {
        name,
        about,
        value: Some(value),
    }
}

static ROOT: Node = Node {
    subs: TOP,
    flags: &[
        flag("--version", "print the version"),
        flag("-v", "print the version"),
        flag("--help", "print the usage"),
        flag("-h", "print the usage"),
    ],
    ..node("vexdom", "")
};

const PAGE_FLAGS: &[Flag] = &[
    opt("--tab", "act on this tab id", Free),
    opt("--max-output", "cap the output at n chars", Free),
    flag("--json", "print {ok, data|error}"),
];

/// Flags that may be given more than once.
const REPEATABLE: &[&str] = &["--param", "--where", "--capability"];

/// The commands `find` can hand its locator to.
const FIND_COMMANDS: &[&str] = &[
    "click",
    "dblclick",
    "fill",
    "type",
    "select",
    "hover",
    "focus",
    "check",
    "uncheck",
    "scrollintoview",
    "upload",
    "download",
    "highlight",
    "drag",
    "get",
    "is",
    "press",
];

const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

const BROWSERS: &[&str] = &[
    "chrome", "edge", "brave", "arc", "vivaldi", "opera", "chromium", "firefox",
];

const JSON: Flag = flag("--json", "print JSON");
const TRUSTED: Flag = flag("--trusted", "send trusted key events");
const FORCE: Flag = flag("--force", "skip actionability checks");
const ONE: &[Value] = &[Free];
const TWO: &[Value] = &[Free, Free];
const FIND: Value = Choices(FIND_COMMANDS);
const FIND_ARGS: &[Value] = &[Free, FIND];
const BUTTONS: Value = Choices(&["left", "middle", "right"]);
const ON_OFF: &[Value] = &[Choices(&["on", "off"])];
const STATE: &[Value] = &[Names(Source::States)];
const AGENT: &[Value] = &[Names(Source::Agents)];
const TASK: &[Value] = &[Names(Source::Tasks)];

const PLUGIN_ADD_FLAGS: &[Flag] = &[
    opt("--name", "the plugin's name", Free),
    opt("--capability", "a capability it offers (repeatable)", Free),
    opt("--capabilities", "its capabilities, comma-separated", Free),
    flag("--no-manifest", "ignore the plugin's manifest"),
];

const RECORD_FLAGS: &[Flag] = &[
    opt("--fps", "frames per second", Free),
    flag("--cursor", "draw the cursor"),
    flag("--contact-sheet", "also write a contact sheet"),
    opt(
        "--contact-sheet-threshold",
        "contact sheet change threshold",
        Free,
    ),
];

const TRACE_SUBS: &[Node] = &[
    Node {
        flags: &[opt(
            "--categories",
            "trace categories, comma-separated",
            Free,
        )],
        ..node("start", "start tracing")
    },
    Node {
        args: &[File],
        ..node("stop", "stop and save the trace")
    },
];

const STORAGE_SUBS: &[Node] = &[
    Node {
        args: TWO,
        ..node("set", "set a key")
    },
    Node {
        args: ONE,
        ..node("remove", "remove a key")
    },
    node("clear", "clear every key"),
];

const MEDIA: &[Value] = &[Choices(&["screen", "print"])];

const TOP: &[Node] = &[
    Node {
        flags: &[
            opt("--browser", "skip the browser question", Choices(BROWSERS)),
            flag("--unpacked", "write the extension folder to load unpacked"),
            opt("--dir", "write the unpacked folder there", Dir),
            flag("--force", "rewrite every file of the unpacked folder"),
            flag("--no-open", "print the add-on link, do not open it"),
            flag("--no-wait", "do not wait for the browser to connect"),
            flag("--no-pair", "mint no pairing code"),
            flag("--restart", "stop the running Bridge first"),
            JSON,
        ],
        ..node("setup", "start the Bridge, add the extension, pair")
    },
    Node {
        flags: &[
            flag("--check", "only say whether a release is out"),
            flag("--local", "re-run setup with this binary"),
            flag("--no-restart", "do not restart the Bridge"),
            JSON,
        ],
        ..node("update", "install the latest signed release")
    },
    Node {
        flags: &[
            flag("--fix", "repair what it safely can"),
            flag("--offline", "skip the network checks"),
            JSON,
        ],
        ..node("doctor", "check this install, the Bridge and browsers")
    },
    Node {
        flags: &[
            flag("--dry-run", "print the plan and stop"),
            flag("--yes", "skip the question"),
            flag("-y", "skip the question"),
            flag("--keep-skills", "leave skills/ behind"),
        ],
        ..node("uninstall", "stop the Bridge and remove what Vexdom wrote")
    },
    Node {
        flags: &[JSON],
        ..node("browsers", "the browsers here, and which are connected")
    },
    node("pair", "issue a one-time pairing code"),
    Node {
        args: &[Choices(&["auto", "ask", "code"])],
        ..node("pairing", "how browsers pair: auto, ask or code")
    },
    Node {
        flags: &[JSON],
        ..node("requests", "browsers and actions waiting for you")
    },
    Node {
        args: ONE,
        flags: &[
            flag("--always", "allow that site from now on"),
            flag("--watch", "ask about each request as it comes"),
            opt("--name", "name the browser", Free),
        ],
        ..node("approve", "allow a waiting browser or action")
    },
    Node {
        args: ONE,
        ..node("deny", "turn a waiting browser or action down")
    },
    node("status", "the Bridge, each paired browser, the agent"),
    node("sessions", "list paired browsers"),
    Node {
        args: TWO,
        ..node("rename", "name a paired browser")
    },
    Node {
        args: ONE,
        ..node("revoke", "unpair one browser, or all of them")
    },
    Node {
        args: AGENT,
        flags: &[JSON],
        subs: &[
            Node {
                args: AGENT,
                ..node("fix", "fix what that agent still needs")
            },
            Node {
                args: AGENT,
                ..node("setup", "fix what that agent still needs")
            },
            Node {
                args: &[Names(Source::Agents), Names(Source::Models)],
                ..node("model", "pin that agent's model")
            },
            Node {
                args: AGENT,
                flags: &[flag("--refresh", "ask the agent's CLI again")],
                ..node("models", "list the models that agent offers")
            },
        ],
        ..node("agent", "show or switch the side panel's agent")
    },
    Node {
        flags: &[JSON],
        ..node("skills", "list the skills the agent can route to")
    },
    Node {
        flags: &[JSON],
        subs: &[
            Node {
                flags: &[opt("--origin", "the site to record", Free)],
                subs: &[
                    node("start", "start recording this tab"),
                    node("stop", "stop recording"),
                    node("status", "whether a recording runs"),
                ],
                ..node("record", "record a task in this tab")
            },
            node("steps", "list what was captured"),
            Node {
                args: ONE,
                flags: &[
                    opt("--from", "first step to keep", Free),
                    opt("--to", "last step to keep", Free),
                    opt("--param", "a parameter k=v (repeatable)", Free),
                    opt("--output", "the step whose answer is the output", Free),
                    opt("--description", "what the flow does", Free),
                    opt("--timeout", "timeout in ms", Free),
                ],
                ..node("save", "keep the recording as a saved flow")
            },
            Node {
                args: ONE,
                flags: &[
                    opt("--param", "a parameter k=v (repeatable)", Free),
                    opt("--origin", "the site to run it on", Free),
                    opt("--allow-write", "allow writes: all or step ids", Free),
                    opt("--where", "keep rows where f=v (repeatable)", Free),
                    opt("--fields", "keep these fields, comma-separated", Free),
                    opt("--limit", "at most n rows", Free),
                    opt("--timeout", "timeout in ms", Free),
                ],
                ..node("run", "rerun a saved flow as direct requests")
            },
            Node {
                flags: &[opt("--origin", "only flows for this site", Free)],
                ..node("list", "list the saved flows")
            },
        ],
        ..node("flow", "record and rerun a task as requests")
    },
    Node {
        flags: &[JSON],
        subs: &[
            Node {
                args: ONE,
                flags: &[
                    opt("--url", "the login page", Free),
                    opt("--username", "the username", Free),
                    opt("--password", "the password", Free),
                    flag("--password-stdin", "read the password from stdin"),
                    opt("--username-selector", "the username field", Free),
                    opt("--password-selector", "the password field", Free),
                    opt("--submit-selector", "the submit button", Free),
                ],
                ..node("save", "save an encrypted login")
            },
            Node {
                args: ONE,
                flags: &[
                    opt("--url", "the login page", Free),
                    flag("--no-navigate", "log in on the current page"),
                    opt(
                        "--credential-provider",
                        "the plugin that resolves the login",
                        Names(Source::Plugins),
                    ),
                    opt("--item", "the provider's item", Free),
                    opt("--timeout", "timeout in ms", Free),
                ],
                ..node("login", "log in with a saved login")
            },
            node("list", "list the saved logins"),
            Node {
                args: ONE,
                ..node("show", "show a saved login")
            },
            Node {
                args: ONE,
                ..node("delete", "delete a saved login")
            },
        ],
        ..node("auth", "saved logins, encrypted in ~/.vexdom/vault")
    },
    Node {
        flags: &[JSON],
        subs: &[
            node("list", "list the plugins"),
            Node {
                args: ONE,
                flags: PLUGIN_ADD_FLAGS,
                ..node("add", "register a plugin")
            },
            Node {
                args: ONE,
                flags: PLUGIN_ADD_FLAGS,
                ..node("install", "register a plugin")
            },
            Node {
                args: &[Names(Source::Plugins)],
                ..node("show", "show a plugin")
            },
            Node {
                args: &[Names(Source::Plugins), Free],
                flags: &[opt("--payload", "the request payload, JSON", Free)],
                ..node("run", "send a plugin a request")
            },
        ],
        ..node("plugin", "the plugins and their capabilities")
    },
    Node {
        flags: &[JSON],
        subs: &[Node {
            args: &[Names(Source::ApprovalHosts)],
            ..node("clear", "forget them, all or one site's")
        }],
        ..node("approvals", "the always-on-this-site approvals")
    },
    Node {
        flags: &[JSON],
        subs: &[
            Node {
                args: TASK,
                ..node("pause", "pause one task, or every task")
            },
            Node {
                args: TASK,
                ..node("resume", "resume one task, or every task")
            },
            Node {
                args: TASK,
                ..node("delete", "delete a task")
            },
        ],
        ..node("tasks", "the scheduled tasks")
    },
    Node {
        flags: &[JSON],
        subs: &[node("clear", "delete all of them")],
        ..node("downloads", "the files captured from pages")
    },
    node("tools", "print the bundled tool manifest"),
    node("logs", "print the Bridge's log"),
    node("start", "bring the Bridge up in the background"),
    node("stop", "stop the Bridge"),
    node("restart", "stop the Bridge and bring up a fresh one"),
    node("token", "print the control token"),
    Node {
        flags: &[opt("--max-output", "cap each answer at n chars", Free)],
        subs: &[Node {
            args: &[Names(Source::McpTools), Free],
            ..node("call", "call one tool like an MCP client")
        }],
        ..node("mcp", "serve MCP over stdio")
    },
    Node {
        args: &[Choices(SHELLS)],
        ..node("completions", "print the completion script for a shell")
    },
    node("help", "print the usage"),
    // Page commands.
    Node {
        flags: &[
            flag("-i", "interactive elements only"),
            flag("--all", "every element"),
            flag("-c", "compact"),
            opt("-d", "depth", Free),
            opt("-s", "the subtree under this selector", Free),
            flag("-u", "show link targets"),
            flag("--full", "the whole tree, not a delta"),
        ],
        ..page("snapshot", "the tab's elements, each with a ref")
    },
    Node {
        args: ONE,
        flags: &[
            opt("--button", "the mouse button", BUTTONS),
            opt("--modifiers", "keys to hold, e.g. ctrl,shift", Free),
            flag("--new-tab", "open the link in a new tab"),
            FORCE,
        ],
        ..page("click", "click an element")
    },
    Node {
        args: ONE,
        flags: &[opt("--modifiers", "keys to hold, e.g. ctrl,shift", Free)],
        ..page("dblclick", "double-click an element")
    },
    Node {
        args: TWO,
        ..page("fill", "replace a field's value")
    },
    Node {
        args: TWO,
        flags: &[
            flag("--clear", "empty it first"),
            opt("--delay", "ms between keys", Free),
            TRUSTED,
        ],
        ..page("type", "type key by key")
    },
    Node {
        args: TWO,
        flags: &[TRUSTED],
        ..page("press", "press a key, e.g. Enter")
    },
    Node {
        args: TWO,
        flags: &[TRUSTED],
        ..page("key", "press a key, e.g. Enter")
    },
    Node {
        args: ONE,
        flags: &[TRUSTED],
        ..page("keydown", "hold a key down")
    },
    Node {
        args: ONE,
        flags: &[TRUSTED],
        ..page("keyup", "release a key")
    },
    Node {
        subs: &[
            Node {
                args: ONE,
                flags: &[TRUSTED],
                ..node("type", "type into the focused element")
            },
            Node {
                args: ONE,
                ..node("inserttext", "insert text into the focused element")
            },
        ],
        ..page("keyboard", "type into the focused element")
    },
    Node {
        args: ONE,
        flags: &[FORCE],
        ..page("hover", "hover an element")
    },
    Node {
        args: ONE,
        ..page("focus", "focus an element")
    },
    Node {
        args: ONE,
        ..page("submit", "submit a form")
    },
    Node {
        args: ONE,
        ..page("highlight", "highlight an element")
    },
    Node {
        args: ONE,
        ..page("scrollintoview", "scroll an element into view")
    },
    Node {
        args: ONE,
        more: Some(Free),
        flags: &[opt(
            "--by",
            "match options by",
            Choices(&["value", "label"]),
        )],
        ..page("select", "choose options in a select")
    },
    Node {
        args: ONE,
        flags: &[FORCE],
        ..page("check", "check a checkbox")
    },
    Node {
        args: ONE,
        flags: &[FORCE],
        ..page("uncheck", "uncheck a checkbox")
    },
    Node {
        args: &[
            Choices(&["up", "down", "left", "right", "top", "bottom"]),
            Free,
        ],
        flags: &[opt("--selector", "scroll this element", Free)],
        ..page("scroll", "scroll the page or an element")
    },
    Node {
        args: TWO,
        ..page("drag", "drag one element onto another")
    },
    Node {
        args: ONE,
        more: Some(File),
        ..page("upload", "set the files of a file input")
    },
    Node {
        args: &[Free, File],
        ..page("download", "click and save the download")
    },
    Node {
        subs: &[
            Node {
                args: ONE,
                ..node("text", "an element's text")
            },
            Node {
                args: ONE,
                ..node("html", "an element's HTML")
            },
            Node {
                args: ONE,
                ..node("value", "a field's value")
            },
            Node {
                args: ONE,
                ..node("count", "how many elements match")
            },
            Node {
                args: ONE,
                ..node("box", "an element's bounding box")
            },
            Node {
                args: ONE,
                ..node("styles", "an element's computed styles")
            },
            Node {
                args: TWO,
                ..node("attr", "an element's attribute")
            },
            node("title", "the page title"),
            node("url", "the page URL"),
        ],
        ..page("get", "read one fact from the page")
    },
    Node {
        subs: &[
            Node {
                args: ONE,
                ..node("visible", "whether it is visible")
            },
            Node {
                args: ONE,
                ..node("enabled", "whether it is enabled")
            },
            Node {
                args: ONE,
                ..node("checked", "whether it is checked")
            },
        ],
        ..page("is", "check an element's state")
    },
    Node {
        flags: &[
            opt("--name", "the accessible name", Free),
            flag("--exact", "match the text exactly"),
        ],
        subs: &[
            Node {
                args: FIND_ARGS,
                ..node("role", "by ARIA role")
            },
            Node {
                args: FIND_ARGS,
                ..node("text", "by text")
            },
            Node {
                args: FIND_ARGS,
                ..node("label", "by label")
            },
            Node {
                args: FIND_ARGS,
                ..node("placeholder", "by placeholder")
            },
            Node {
                args: FIND_ARGS,
                ..node("alt", "by alt text")
            },
            Node {
                args: FIND_ARGS,
                ..node("title", "by title")
            },
            Node {
                args: FIND_ARGS,
                ..node("testid", "by test id")
            },
            Node {
                args: FIND_ARGS,
                ..node("first", "the first match of a selector")
            },
            Node {
                args: FIND_ARGS,
                ..node("last", "the last match of a selector")
            },
            Node {
                args: &[Free, Free, FIND],
                ..node("nth", "the nth match of a selector")
            },
        ],
        ..page("find", "run a command on a located element")
    },
    Node {
        args: ONE,
        flags: &[
            opt(
                "--state",
                "the state to wait for",
                Choices(&["attached", "visible", "hidden", "detached"]),
            ),
            opt("--timeout", "timeout in ms", Free),
            opt("--text", "wait for this text", Free),
            opt("--url", "wait for this URL", Free),
            opt("--load", "wait for this load state", Free),
            opt("--fn", "wait until this JavaScript is true", Free),
            flag("--download", "wait for a download"),
        ],
        ..page("wait", "wait for time, an element or a condition")
    },
    Node {
        args: ONE,
        flags: &[opt("--init-script", "add an init script first", File)],
        ..page("open", "go to a URL")
    },
    Node {
        args: ONE,
        flags: &[opt("--init-script", "add an init script first", File)],
        ..page("goto", "go to a URL")
    },
    Node {
        args: ONE,
        flags: &[opt("--init-script", "add an init script first", File)],
        ..page("navigate", "go to a URL")
    },
    page("back", "go back"),
    page("forward", "go forward"),
    page("reload", "reload the page"),
    Node {
        args: ONE,
        ..page("pushstate", "change the URL without loading")
    },
    Node {
        args: ONE,
        subs: &[
            Node {
                args: ONE,
                flags: &[opt("--label", "label the tab", Free)],
                ..node("new", "open a new tab")
            },
            Node {
                args: ONE,
                ..node("close", "close a tab")
            },
            Node {
                args: &[Choices(&["live", "throttled", "frozen"]), Free],
                ..node("keep", "keep a background tab live")
            },
        ],
        ..page("tab", "list or switch tabs")
    },
    Node {
        subs: &[Node {
            args: ONE,
            ..node("new", "open a new window")
        }],
        ..page("window", "open a window")
    },
    Node {
        args: &[Choices(&["main"])],
        ..page("frame", "enter an iframe, or go back to main")
    },
    Node {
        args: ONE,
        flags: &[
            opt("-b", "the script, base64", Free),
            opt("--base64", "the script, base64", Free),
            flag("--stdin", "read the script from stdin"),
        ],
        ..page("eval", "run JavaScript in the page")
    },
    Node {
        subs: &[
            Node {
                args: ONE,
                subs: STORAGE_SUBS,
                ..node("local", "localStorage")
            },
            Node {
                args: ONE,
                subs: STORAGE_SUBS,
                ..node("session", "sessionStorage")
            },
        ],
        ..page("storage", "read or change web storage")
    },
    Node {
        flags: &[opt("--url", "cookies for this URL", Free)],
        subs: &[
            Node {
                args: ONE,
                ..node("get", "list cookies, or one")
            },
            Node {
                args: TWO,
                flags: &[
                    opt("--domain", "the cookie's domain", Free),
                    opt("--path", "the cookie's path", Free),
                    flag("--secure", "secure only"),
                    flag("--http-only", "HTTP only"),
                    opt("--same-site", "Strict, Lax or None", Free),
                    opt("--expires", "expiry, Unix seconds", Free),
                    opt("--curl", "set the cookies of a curl cookie file", File),
                ],
                ..node("set", "set a cookie")
            },
            Node {
                args: ONE,
                ..node("clear", "clear cookies, or one")
            },
        ],
        ..page("cookies", "read or change cookies")
    },
    Node {
        subs: &[
            node("read", "read the clipboard"),
            Node {
                args: ONE,
                ..node("write", "write text to the clipboard")
            },
            node("copy", "copy the selection"),
            node("paste", "paste into the focused element"),
        ],
        ..page("clipboard", "read or write the clipboard")
    },
    Node {
        subs: &[
            Node {
                args: ONE,
                flags: &[opt("--frame", "the frame id", Free)],
                ..node("list", "list the page's WebMCP tools")
            },
            Node {
                args: ONE,
                flags: &[
                    opt("--params", "the input, JSON or @file", Free),
                    opt("--frame", "the frame id", Free),
                    flag("--detach", "return an id, not the result"),
                ],
                ..node("invoke", "call a WebMCP tool")
            },
            Node {
                args: ONE,
                flags: &[opt("--wait", "wait up to ms", Free)],
                ..node("result", "a detached call's result")
            },
            Node {
                args: ONE,
                ..node("cancel", "cancel a detached call")
            },
        ],
        ..page("webmcp", "the page's WebMCP tools")
    },
    Node {
        args: ONE,
        flags: &[
            flag("--outline", "headings only"),
            opt("--filter", "sections mentioning this text", Free),
            opt("--target", "one element", Free),
            opt("--offset", "start at this char", Free),
            flag("--raw", "the body as served"),
            flag("--require-md", "fail unless served as markdown"),
            opt("--llms", "read llms.txt", Choices(&["index", "full"])),
            opt("--timeout", "timeout in ms", Free),
        ],
        ..page("read", "the page as markdown")
    },
    Node {
        flags: &[
            opt("-o", "write it to this file", File),
            opt("--screenshot-dir", "write it in this directory", Dir),
            opt(
                "--screenshot-format",
                "the image format",
                Choices(&["png", "jpeg"]),
            ),
            opt("--screenshot-quality", "JPEG quality, 0-100", Free),
            flag("--full", "the whole page"),
            opt("-s", "only this element", Free),
            flag("--if-changed", "skip it if nothing changed"),
            opt("--threshold", "change threshold, 0-1", Free),
            flag("--annotate", "draw each ref on its element"),
        ],
        ..page("screenshot", "capture the tab")
    },
    Node {
        subs: &[
            Node {
                flags: &[
                    opt("--baseline", "compare with this saved snapshot", File),
                    opt("-s", "the subtree under this selector", Free),
                    flag("-c", "compact"),
                    opt("-d", "depth", Free),
                ],
                ..node("snapshot", "what changed since a snapshot")
            },
            Node {
                flags: &[
                    opt("--baseline", "compare with this image", File),
                    opt("-o", "write the diff image here", File),
                    opt("--threshold", "change threshold, 0-1", Free),
                    opt("-s", "only this element", Free),
                    flag("--full", "the whole page"),
                ],
                ..node("screenshot", "what changed since a screenshot")
            },
            Node {
                args: TWO,
                flags: &[
                    flag("--screenshot", "compare screenshots too"),
                    flag("--full", "the whole page"),
                ],
                ..node("url", "compare two URLs")
            },
        ],
        ..page("diff", "what changed")
    },
    Node {
        subs: &[
            Node {
                args: STATE,
                ..node("save", "save cookies and web storage")
            },
            Node {
                args: STATE,
                ..node("load", "load a saved state")
            },
            Node {
                args: STATE,
                ..node("show", "show a saved state")
            },
            node("list", "list the saved states"),
            Node {
                args: &[Names(Source::States), Free],
                ..node("rename", "rename a saved state")
            },
            Node {
                args: STATE,
                flags: &[flag("--all", "every saved state")],
                ..node("clear", "delete a saved state")
            },
            Node {
                flags: &[opt("--older-than", "older than this many days", Free)],
                ..node("clean", "delete old saved states")
            },
        ],
        ..page("state", "saved cookies and web storage")
    },
    Node {
        flags: &[
            opt("--level", "only this level", Free),
            opt("--contains", "only messages with this text", Free),
            opt("--limit", "at most n messages", Free),
            flag("--clear", "clear them"),
        ],
        ..page("console", "the page's console messages")
    },
    Node {
        flags: &[flag("--clear", "clear them")],
        ..page("errors", "the page's uncaught errors")
    },
    Node {
        subs: &[
            Node {
                flags: &[
                    opt("--filter", "URLs containing this text", Free),
                    opt("--type", "this resource type", Free),
                    opt("--method", "this HTTP method", Free),
                    opt("--status", "this status", Free),
                    flag("--clear", "clear them"),
                ],
                ..node("requests", "the page's requests")
            },
            Node {
                args: ONE,
                ..node("request", "one request in full")
            },
            Node {
                args: ONE,
                flags: &[
                    flag("--abort", "fail matching requests"),
                    opt("--body", "answer with this body, JSON or @file", Free),
                    opt("--status", "answer with this status", Free),
                    opt("--content-type", "answer with this content type", Free),
                    opt("--allow-origin", "answer with this CORS origin", Free),
                    opt("--resource-type", "only these types, comma-separated", Free),
                ],
                ..node("route", "intercept matching requests")
            },
            Node {
                args: ONE,
                ..node("unroute", "stop intercepting")
            },
            Node {
                subs: &[
                    Node {
                        flags: &[opt(
                            "--content",
                            "which bodies to keep",
                            Choices(&["all", "text", "none"]),
                        )],
                        ..node("start", "start recording a HAR")
                    },
                    Node {
                        args: &[File],
                        ..node("stop", "stop and save the HAR")
                    },
                ],
                ..node("har", "record a HAR")
            },
        ],
        ..page("network", "the page's network traffic")
    },
    Node {
        subs: &[
            Node {
                args: TWO,
                flags: &[
                    flag("--human", "move like a person"),
                    opt("--seed", "random seed", Free),
                    opt("--steps", "intermediate steps", Free),
                    opt("--duration", "duration in ms", Free),
                ],
                ..node("move", "move the mouse")
            },
            Node {
                args: &[BUTTONS],
                ..node("down", "press a button")
            },
            Node {
                args: &[BUTTONS],
                ..node("up", "release a button")
            },
            Node {
                args: TWO,
                ..node("wheel", "turn the wheel")
            },
        ],
        ..page("mouse", "drive the mouse")
    },
    Node {
        subs: &[
            node("status", "the open dialog, if any"),
            Node {
                args: ONE,
                ..node("accept", "accept it, with prompt text")
            },
            node("dismiss", "dismiss it"),
            Node {
                args: ON_OFF,
                flags: &[
                    flag("--alert", "alerts only"),
                    flag("--beforeunload", "beforeunload only"),
                ],
                ..node("auto", "accept dialogs by themselves")
            },
        ],
        ..page("dialog", "the page's JavaScript dialogs")
    },
    Node {
        subs: &[
            Node {
                args: TWO,
                flags: &[
                    opt("--scale", "device scale factor", Free),
                    flag("--mobile", "emulate a mobile device"),
                ],
                ..node("viewport", "set the viewport size")
            },
            Node {
                args: ONE,
                ..node("device", "emulate a device")
            },
            Node {
                args: TWO,
                flags: &[opt("--accuracy", "accuracy in meters", Free)],
                ..node("geo", "set the geolocation")
            },
            Node {
                args: ON_OFF,
                ..node("offline", "go offline or online")
            },
            Node {
                args: ONE,
                ..node("headers", "extra request headers, JSON")
            },
            Node {
                args: TWO,
                ..node("credentials", "HTTP auth credentials")
            },
            Node {
                args: MEDIA,
                flags: &[
                    opt("--color-scheme", "light or dark", Free),
                    opt("--reduced-motion", "reduce or no-preference", Free),
                ],
                ..node("media", "emulate media")
            },
            node("reset", "undo every emulation"),
        ],
        ..page("set", "emulate a device, place or network")
    },
    Node {
        args: &[File],
        flags: &[
            flag("--landscape", "landscape"),
            flag("--no-background", "skip backgrounds"),
            flag("--css-page-size", "use the CSS page size"),
        ],
        ..page("pdf", "print the page to PDF")
    },
    Node {
        subs: &[
            Node {
                args: &[File],
                flags: RECORD_FLAGS,
                ..node("start", "start recording a video")
            },
            Node {
                args: &[File],
                flags: RECORD_FLAGS,
                ..node("restart", "start a new video")
            },
            node("stop", "stop and save the video"),
        ],
        ..page("record", "record the tab as a video")
    },
    Node {
        subs: TRACE_SUBS,
        ..page("trace", "record a performance trace")
    },
    Node {
        subs: TRACE_SUBS,
        ..page("profiler", "record a performance trace")
    },
    Node {
        flags: &[flag("--reload", "reload the page first")],
        ..page("vitals", "the page's web vitals")
    },
    Node {
        subs: &[
            node("install", "install the React hook"),
            node("tree", "the component tree"),
            Node {
                args: ONE,
                ..node("inspect", "one component")
            },
            Node {
                subs: &[
                    node("start", "start counting renders"),
                    node("stop", "stop and report"),
                ],
                ..node("renders", "count renders")
            },
            Node {
                flags: &[flag("--only-dynamic", "only dynamic boundaries")],
                ..node("suspense", "the Suspense boundaries")
            },
        ],
        ..page("react", "inspect a React app")
    },
    Node {
        flags: &[
            opt("--tags", "rule tags, comma-separated", Free),
            opt("--selector", "only this element", Free),
        ],
        ..page("a11y", "check accessibility")
    },
    Node {
        subs: &[
            Node {
                args: &[File],
                ..node("add", "add a script to run on every load")
            },
            Node {
                args: ONE,
                ..node("remove", "remove one")
            },
            node("list", "list them"),
        ],
        ..page("initscript", "scripts run before each page loads")
    },
    Node {
        args: ONE,
        more: Some(Free),
        flags: &[
            flag("--bail", "stop at the first failure"),
            flag("--omit-command", "do not echo each command"),
        ],
        ..page("batch", "run several commands in one go")
    },
];

#[derive(Debug, PartialEq)]
enum Completion {
    /// `(value, about)`.
    Words(Vec<(String, String)>),
    Files,
    Dirs,
}

/// Mirrors `Args::positional`: `--x` and `-x` (not `-5`) are flags.
fn is_flag(word: &str) -> bool {
    word.starts_with("--")
        || (word.len() == 2
            && word.starts_with('-')
            && !word[1..].chars().all(|c| c.is_ascii_digit()))
}

/// The flags in scope at `path`.
fn flags_of(path: &[&'static Node]) -> Vec<&'static Flag> {
    if path.len() == 1 {
        return ROOT.flags.iter().collect();
    }
    let mut flags: Vec<&'static Flag> = path[1..].iter().flat_map(|node| node.flags).collect();
    if path[1].page {
        flags.extend(PAGE_FLAGS);
    }
    flags
}

/// What to offer for word `cword` of `words` (the words after `vexdom`).
fn complete(cword: usize, words: &[&str]) -> Completion {
    let current = words.get(cword).copied().unwrap_or("");
    let done = &words[..cword.min(words.len())];
    let mut path: Vec<&'static Node> = vec![&ROOT];
    let mut index = 0;
    let mut pending: Option<Value> = None;
    let mut verbatim = false;
    let mut used: Vec<&str> = Vec::new();
    for &word in done {
        if pending.take().is_some() {
            continue;
        }
        if !verbatim && word == "--" {
            verbatim = true;
            continue;
        }
        if !verbatim && is_flag(word) {
            used.push(word);
            pending = flags_of(&path)
                .into_iter()
                .find(|flag| flag.name == word)
                .and_then(|flag| flag.value);
            continue;
        }
        let deepest = path[path.len() - 1];
        if index == 0
            && !verbatim
            && let Some(sub) = deepest.subs.iter().find(|sub| sub.name == word)
        {
            path.push(sub);
            continue;
        }
        index += 1;
    }
    if let Some(value) = pending {
        return values(value, current);
    }
    if !verbatim && current.starts_with('-') {
        return Completion::Words(
            flags_of(&path)
                .into_iter()
                .filter(|flag| flag.name.starts_with(current))
                .filter(|flag| !used.contains(&flag.name) || REPEATABLE.contains(&flag.name))
                .map(|flag| (flag.name.to_owned(), flag.about.to_owned()))
                .collect(),
        );
    }
    let deepest = path[path.len() - 1];
    let arg = deepest.args.get(index).copied().or(deepest.more);
    if index == 0 && !deepest.subs.is_empty() {
        let mut words: Vec<(String, String)> = deepest
            .subs
            .iter()
            .filter(|sub| sub.name.starts_with(current))
            .map(|sub| (sub.name.to_owned(), sub.about.to_owned()))
            .collect();
        if let Some(value @ (Choices(_) | Names(_))) = arg
            && let Completion::Words(more) = values(value, current)
        {
            words.extend(more);
        }
        return Completion::Words(words);
    }
    match arg {
        Some(value) => values(value, current),
        None => Completion::Words(Vec::new()),
    }
}

fn values(value: Value, current: &str) -> Completion {
    match value {
        Free => Completion::Words(Vec::new()),
        File => Completion::Files,
        Dir => Completion::Dirs,
        Choices(list) => Completion::Words(
            list.iter()
                .filter(|choice| choice.starts_with(current))
                .map(|choice| ((*choice).to_owned(), String::new()))
                .collect(),
        ),
        Names(source) => Completion::Words(
            names(source)
                .into_iter()
                .filter(|(name, _)| name.starts_with(current))
                .collect(),
        ),
    }
}

/// Names read from local files; never the Bridge, never stderr.
fn names(source: Source) -> Vec<(String, String)> {
    let found: Vec<(String, String)> = match source {
        Source::States => super::state::saved_files()
            .iter()
            .filter_map(|path| Some((path.file_stem()?.to_str()?.to_owned(), String::new())))
            .collect(),
        Source::Plugins => vexdom_core::plugins::configured()
            .0
            .into_iter()
            .map(|plugin| (plugin.name, plugin.capabilities.join(", ")))
            .collect(),
        Source::Tasks => vexdom_core::schedules::read_schedules()
            .tasks
            .iter()
            .filter_map(|task| {
                let id = task.get("id")?.as_str()?;
                let name = task.get("name").and_then(serde_json::Value::as_str);
                Some((id.to_owned(), name.unwrap_or_default().to_owned()))
            })
            .collect(),
        Source::Agents => AGENT_LIST
            .iter()
            .map(|agent| (agent.kind.to_owned(), agent.label.to_owned()))
            .collect(),
        Source::Models => AGENT_LIST
            .iter()
            .flat_map(|agent| agent.models)
            .map(|model| ((*model).to_owned(), String::new()))
            .collect(),
        Source::McpTools => crate::mcp::host::tool_definitions()
            .iter()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?;
                Some((
                    name.to_owned(),
                    tool["title"].as_str().unwrap_or_default().to_owned(),
                ))
            })
            .collect(),
        Source::ApprovalHosts => {
            let mut hosts: Vec<String> = Vec::new();
            for grant in vexdom_core::approvals::list_grants() {
                if !hosts.contains(&grant.host) {
                    hosts.push(grant.host);
                }
            }
            hosts
                .into_iter()
                .map(|host| (host, String::new()))
                .collect()
        }
    };
    found
        .into_iter()
        .filter(|(name, _)| !name.contains(['\t', '\n']))
        .map(|(name, about)| (name, about.replace(['\t', '\n'], " ")))
        .collect()
}

/// `vexdom __complete <cword> <words…>`: a directive line (`:words`, `:files` or `:dirs`), then
/// for `:words` one `value` or `value\tabout` per line.
pub(super) fn complete_command(argv: &[String]) -> ExitCode {
    let completion = match argv.first().and_then(|cword| cword.parse::<usize>().ok()) {
        Some(cword) => {
            let words: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
            complete(cword, &words)
        }
        None => Completion::Words(Vec::new()),
    };
    let out = match completion {
        Completion::Files => ":files\n".to_owned(),
        Completion::Dirs => ":dirs\n".to_owned(),
        Completion::Words(words) => {
            let mut out = String::from(":words\n");
            for (value, about) in words {
                out.push_str(&value);
                if !about.is_empty() {
                    out.push('\t');
                    out.push_str(&about);
                }
                out.push('\n');
            }
            out
        }
    };
    // A shell that stops reading early is no error worth reporting.
    let _ = std::io::stdout().write_all(out.as_bytes());
    ExitCode::SUCCESS
}

/// `vexdom completions <shell>`: the script that wires Tab to `vexdom __complete`.
pub(super) fn print_script(shell: Option<&str>) -> ExitCode {
    let script = match shell {
        Some("bash") => include_str!("completions/vexdom.bash"),
        Some("zsh") => include_str!("completions/vexdom.zsh"),
        Some("fish") => include_str!("completions/vexdom.fish"),
        Some("powershell") => include_str!("completions/vexdom.ps1"),
        None => return fail("Usage: vexdom completions bash|zsh|fish|powershell"),
        Some(other) => {
            return fail(&format!(
                "Unknown shell \"{other}\": use bash, zsh, fish or powershell"
            ));
        }
    };
    let _ = std::io::stdout().write_all(script.as_bytes());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::super::{VALUE_FLAGS, page, verbs};
    use super::*;

    /// The top-level commands `run` dispatches itself, not through `page::run`.
    const DISPATCHED: &[&str] = &[
        "setup",
        "update",
        "doctor",
        "uninstall",
        "browsers",
        "pair",
        "pairing",
        "requests",
        "approve",
        "deny",
        "status",
        "sessions",
        "rename",
        "revoke",
        "agent",
        "skills",
        "flow",
        "auth",
        "plugin",
        "approvals",
        "tasks",
        "downloads",
        "tools",
        "logs",
        "start",
        "stop",
        "restart",
        "token",
        "mcp",
        "completions",
        "help",
    ];

    fn words(completion: Completion) -> Vec<String> {
        match completion {
            Completion::Words(words) => words.into_iter().map(|(value, _)| value).collect(),
            other => panic!("expected words, got {other:?}"),
        }
    }

    fn value_flags(nodes: &'static [Node], out: &mut Vec<&'static str>) {
        for node in nodes {
            out.extend(
                node.flags
                    .iter()
                    .filter(|flag| flag.value.is_some())
                    .map(|flag| flag.name),
            );
            value_flags(node.subs, out);
        }
    }

    #[test]
    fn commands_by_prefix() {
        let found = words(complete(0, &["cl"]));
        assert!(found.contains(&"click".to_owned()));
        assert!(found.contains(&"clipboard".to_owned()));
        assert!(found.iter().all(|word| word.starts_with("cl")));
    }

    #[test]
    fn page_commands_take_page_flags() {
        let found = words(complete(1, &["snapshot", "-"]));
        for wanted in ["-c", "--all", "--tab", "--json"] {
            assert!(found.contains(&wanted.to_owned()), "{wanted} in {found:?}");
        }
        assert!(!found.contains(&"--version".to_owned()));
    }

    #[test]
    fn flag_values() {
        assert_eq!(
            words(complete(2, &["screenshot", "--screenshot-format", ""])),
            ["png", "jpeg"]
        );
        assert_eq!(complete(2, &["screenshot", "-o", ""]), Completion::Files);
        assert_eq!(
            complete(3, &["upload", "@e1", "a.txt", ""]),
            Completion::Files
        );
        assert_eq!(complete(2, &["setup", "--dir", ""]), Completion::Dirs);
    }

    #[test]
    fn subcommands_and_their_arguments() {
        assert_eq!(words(complete(1, &["tab", ""])), ["new", "close", "keep"]);
        assert_eq!(
            words(complete(2, &["tab", "keep", ""])),
            ["live", "throttled", "frozen"]
        );
    }

    #[test]
    fn a_value_flag_eats_its_word() {
        assert_eq!(
            words(complete(3, &["scroll", "--selector", "main", ""])),
            ["up", "down", "left", "right", "top", "bottom"]
        );
    }

    #[test]
    fn used_flags_drop_out_but_repeatable_ones_stay() {
        let found = words(complete(2, &["screenshot", "--full", "--f"]));
        assert!(!found.contains(&"--full".to_owned()));
        let found = words(complete(4, &["flow", "run", "--param", "a=1", "--pa"]));
        assert!(found.contains(&"--param".to_owned()));
    }

    #[test]
    fn nothing_is_a_flag_after_double_dash() {
        assert_eq!(
            complete(2, &["eval", "--", "-"]),
            Completion::Words(Vec::new())
        );
    }

    #[test]
    fn page_commands_match_the_dispatcher() {
        for verb in page::VERBS
            .iter()
            .chain(verbs::VERBS)
            .chain(page::OWN_VERBS)
        {
            let node = TOP.iter().find(|node| node.name == *verb);
            assert!(node.is_some_and(|node| node.page), "{verb} is a page node");
        }
        for node in TOP {
            if node.page {
                assert!(
                    page::is_page_command(node.name),
                    "{} runs as a page command",
                    node.name
                );
            } else {
                assert!(
                    DISPATCHED.contains(&node.name),
                    "{} is dispatched by run",
                    node.name
                );
            }
        }
    }

    #[test]
    fn value_flags_match_the_parser() {
        let mut spec = Vec::new();
        value_flags(TOP, &mut spec);
        spec.extend(
            PAGE_FLAGS
                .iter()
                .filter(|flag| flag.value.is_some())
                .map(|flag| flag.name),
        );
        for name in &spec {
            // `setup` parses its own argv.
            assert!(
                VALUE_FLAGS.contains(name) || *name == "--dir",
                "{name} is in VALUE_FLAGS"
            );
        }
        for name in VALUE_FLAGS {
            // `--ref` is listed but read by no command.
            assert!(
                *name == "--ref" || spec.contains(name),
                "{name} is in the spec"
            );
        }
    }

    #[test]
    fn find_offers_every_element_verb() {
        for verb in page::ELEMENT_VERBS {
            assert!(FIND_COMMANDS.contains(verb), "{verb} in FIND_COMMANDS");
        }
    }
}
