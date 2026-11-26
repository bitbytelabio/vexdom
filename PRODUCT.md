# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- The desktop app is native macOS (GPUI, `desktop/`); the extension pages are web. Impeccable's
platform values have no macOS-native entry, so `web` stands for the extension and the desktop app
follows macOS conventions by evidence. -->

## Users

Developers on macOS who already use Claude Code (or an MCP client) and want it to drive their real,
signed-in browser. The desktop app is for keeping the local Bridge running, pairing browsers,
choosing the agent's model, and seeing notices, without a terminal.

## Product Purpose

vexdom lets an agent work in the browser you are already signed in to: the extension acts on the
page, the Bridge (`vexdom`, one Rust binary: daemon, CLI, MCP server, native host) connects it to
Claude Code from the side panel or to any MCP client. Success is a task done in your own session
with fewer tokens than a screenshot-and-guess agent spends.

## Positioning

- Your own logged-in browser, not a headless or cloud one.
- Everything on 127.0.0.1: no account, no cloud service, no telemetry.
- Ref-addressed, incremental page snapshots: −46 %, −57 %, −63 % tokens per task in the live
  `claude -p` bench (`bench/tokens/live.md`).

## Operating Context

- macOS first (ADR-026); Linux builds; Windows is not supported.
- Chrome, Edge, Brave, Arc, Vivaldi, Opera, Chromium and Firefox; the extension is loaded unpacked
  or installed as the signed Firefox add-on.
- Pairing: by a code (`vexdom pair`), automatically, or by asking (a browser waits until someone
  allows it in the app or with `vexdom approve`).
- The desktop app shells out to the `vexdom` CLI for start, stop, setup and the model, and talks to
  the Bridge's control socket for status, pairing requests, sessions and notices.

## Capabilities and Constraints

- Agent runner: Claude Code only for now (other runners deferred).
- Desktop app MVP: Bridge on/off, add the extension per browser, paired browsers and unpair,
  pairing requests (allow/deny with a name), Claude Code status and model, notices with OS
  notifications.

## Brand Commitments

- Name: vexdom, lowercase in the CLI.
- No logo yet.

## Evidence on Hand

- Token measurements: `bench/tokens/live.md`, `bench/tokens/baseline.md`.
- No customers, testimonials or press; none may be invented.

## Product Principles

1. Local and owned: nothing leaves the machine unless the user's agent sends it.
2. The user's browser stays the user's: pairing is explicit, every browser can be unpaired.
3. One source of truth: the CLI and the Bridge own behaviour; the apps show and trigger it.
4. Spend fewer tokens: show the agent what it needs, not the whole page.
