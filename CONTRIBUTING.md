# Contributing to Vexdom

Bug reports, capability ideas and pull requests are welcome. This page covers where each kind of
contribution goes, the one check every pull request must pass, and where the deeper guides live.

## Ways in

| You have | Do this |
| --- | --- |
| A bug | [Open a bug report](https://github.com/bitbytelabio/vexdom/issues/new?template=bug_report.yml) with `vexdom status` output |
| A capability idea | [Open a proposal](https://github.com/bitbytelabio/vexdom/issues/new?template=capability.yml) before writing code |
| A security problem | **Not an issue.** [Report it privately](SECURITY.md): Vexdom holds live browser sessions |
| A docs fix | Edit under `docs/` and open a PR straight away; no issue needed |
| A small code fix | A PR straight away is fine too |

Propose before you code anything that adds surface. A capability becomes a page tool, for the
side panel's agent and for MCP clients alike, the moment it lands in the registry, so its name,
shape and guardrails are API decisions worth two paragraphs of discussion before they become a diff.

## Setup

The repository root is the Rust workspace (the Bridge); the extension is the Bun project in
`packages/extension/`, and every `bun` command below runs there. One command builds everything
with nothing on your `PATH` but Bun 1.4.5 and the Rust tools:

```sh
cd packages/extension
bun run setup
```

Then `bun run dev` launches a throwaway Chrome profile with hot reload. The full build topology,
the daemon's lifecycle, and every command live in the
[internals contributing guide](docs/internals/contributing.md).

## The required check

```sh
bun run check
```

It builds ext-core, runs the Rust workspace's `cargo fmt --check`, clippy and tests (`bun run
bridge:check`), the TypeScript type check, and the test suite with its coverage floors. CI runs the
same command on your pull request, so green locally means green in CI.

If you touched the action registry, also run `bun run tools:emit` (the Bridge embeds the manifest it
writes to `crates/vexdom-core/assets/`; a test fails while they drift), then check
`vexdom tools` against [docs/reference/tools.md](docs/reference/tools.md).

## Tests

A test sits next to the module it tests, as `<module>.test.ts`, and runs with
[Vitest](https://vitest.dev):

```sh
bun run test                          # everything
bun run test src/lib/bridge        # one directory or file
bun run coverage                      # everything, then coverage by area against its floors
```

Where a test lives decides what it runs in:

| Where | Runs in |
| --- | --- |
| `packages/extension/src/lib/**` | Node, with WXT's fake `browser` |
| `packages/extension/src/lib/actions/page/**` | happy-dom |
| `packages/extension/src/lib/**/*.integration.test.ts` | Node, against the Rust Bridge binary (`target/debug/vexdom`) and ext-core's node build, in a throwaway home |
| `app/**`, `crates/**` | `cargo test` (the Bridge, the CLI, the MCP server, ext-core natively) |

Never put a test under `packages/extension/src/extension/entrypoints/`, because WXT builds every file there as an
entrypoint. The floors in [vitest.coverage.ts](packages/extension/vitest.coverage.ts) only ever go up: raise one when
your tests lift an area, and never lower one to get a build through.

## Adding a capability

The short version: one action module in `packages/extension/src/lib/actions/page/`, one line in
[the registry](packages/extension/src/lib/actions/registry.ts), `bun run tools:emit`, and the Bridge publishes it as a page tool over MCP.
The extension and the MCP server build from the same registry, so a tool can never describe
something the browser cannot do. The long version, including the four conventions that are
load-bearing at runtime, is in the
[internals guide](docs/internals/contributing.md#adding-a-capability).

Think about the guardrail while you are there: anything that commits something, sends data
somewhere, or acts on another site's security control should carry a rule in the
[policy](docs/guide/approvals.md), not a scattered check.

## Pull requests

- **One concern per PR.** A fix and a refactor are two PRs.
- **Conventional commits**, subject line first: `feat(actions): …`, `fix(daemon): …`,
  `docs: …`. Look at `git log --oneline` and match it.
- **Docs travel with the change.** A new capability without its `docs/` page is half a PR.
- CI runs `bun run check` and both builds on every PR. A red check is yours to fix, but ask if
  the failure makes no sense.

## Conduct

Be the person you would want reviewing your first PR. The specifics are in the
[code of conduct](CODE_OF_CONDUCT.md).

## License

Vexdom is licensed under the [Apache License 2.0](LICENSE). Contributions are accepted under the same terms:
inbound = outbound, no CLA.
