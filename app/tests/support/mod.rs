//! A scratch Bridge in this process, a Rust fake extension paired with it, and `vexdom mcp` over
//! stdio against both: what the app's MCP tests and the token bench drive.

#![allow(dead_code)]

#[path = "../../../crates/vexdom-daemon/tests/support/fake_extension.rs"]
pub mod fake_extension;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use fake_extension::{Credential, Outcome, Shared};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// One `vexdom mcp` process; every line sent and received is kept, `> ` and `< ` prefixed, in the
/// golden-session format of bench/tokens/golden/mcp/.
pub struct Session {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next: u64,
    pub transcript: Vec<String>,
}

impl Session {
    pub async fn start(home: &Path) -> Self {
        Self::start_with(home, &[]).await
    }

    /// A session whose server also gets `env`.
    pub async fn start_with(home: &Path, env: &[(&str, &str)]) -> Self {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vexdom"))
            .arg("mcp")
            .env("VEXDOM_HOME", home)
            .env("HOME", home)
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("vexdom mcp starts");
        let stdin = child.stdin.take().expect("piped stdin");
        let lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        Self {
            _child: child,
            stdin,
            lines,
            next: 0,
            transcript: Vec::new(),
        }
    }

    async fn send(&mut self, message: Value) {
        let line = message.to_string();
        self.transcript.push(format!("> {line}"));
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("stdin is open");
        self.stdin.flush().await.expect("stdin is open");
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}))
            .await;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(20), self.lines.next_line())
                .await
                .expect("the MCP server answered in time")
                .expect("stdout reads")
                .expect("the MCP server is still running");
            self.transcript.push(format!("< {line}"));
            let message: Value = serde_json::from_str(&line).expect("a JSON-RPC message");
            if message["id"] == json!(self.next) {
                return message;
            }
        }
    }

    pub async fn notify(&mut self, method: &str) {
        self.send(json!({"jsonrpc": "2.0", "method": method})).await;
    }

    pub async fn initialize(&mut self) -> Value {
        let init = self
            .request(
                "initialize",
                json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
            )
            .await;
        self.notify("notifications/initialized").await;
        init
    }

    /// The first text block of a tool call's reply.
    pub async fn call(&mut self, tool: &str, arguments: Value) -> String {
        let reply = self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

/// A fresh `VEXDOM_HOME`, `HOME` and `VEXDOM_PORTS=0` for a test binary with one test.
#[allow(unsafe_code)]
pub fn scratch(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("vexdom-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // SAFETY: each test binary using this has one test, so nothing reads the environment concurrently.
    unsafe {
        std::env::set_var("VEXDOM_HOME", &home);
        std::env::set_var("VEXDOM_PORTS", "0");
        std::env::set_var("HOME", &home);
    }
    home
}

/// Starts a Bridge in this process and pairs the fake extension, which answers from `canned`.
pub async fn paired_bridge(canned: &Shared) -> vexdom_daemon::Daemon {
    let daemon = vexdom_daemon::Daemon::start(false)
        .await
        .expect("the Bridge starts");
    let control = vexdom_link::connect_control(daemon.port(), daemon.token())
        .await
        .expect("control")
        .0;
    let pair = control.pair_request().send().promise.await.expect("a code");
    let code = pair
        .get()
        .expect("reply")
        .get_code()
        .expect("code")
        .to_string()
        .expect("utf-8");
    let Outcome::Welcome { .. } =
        fake_extension::connect(daemon.port(), Credential::Code(&code), Rc::clone(canned))
            .await
            .expect("the socket opens")
    else {
        panic!("pairing failed");
    };
    daemon
}
