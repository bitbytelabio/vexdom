mod cli;
mod mcp;
mod native_host;
// Vendored from agent-browser; kept close to its source.
#[allow(clippy::collapsible_if)]
mod read_url;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    // The names the npm package installs beside `vexdom` (ADR-004): `vexdom-mcpd` is the daemon,
    // `vexdom-mcp` the MCP server.
    match argv
        .first()
        .and_then(|zero| Path::new(zero).file_stem())
        .and_then(|stem| stem.to_str())
    {
        Some("vexdom-mcpd") => {
            return vexdom_daemon::run(vexdom_daemon::Options {
                no_idle_exit: argv.iter().any(|arg| arg == "--no-idle-exit"),
            });
        }
        Some("vexdom-mcp") if argv.len() == 1 => {
            return mcp::run(mcp::Options { max_output: None });
        }
        _ => {}
    }
    cli::run(argv)
}
