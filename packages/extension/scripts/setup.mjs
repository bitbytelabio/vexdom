#!/usr/bin/env bun
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// The extension build compiles ext-core to wasm, so cargo, the wasm32 target, boltffi and
// wasm-opt come first (CONTRIBUTING.md); the Bridge is the Rust workspace at the repository root.
// process.execPath is the Bun running this script, so the version on PATH does not matter.
const STEPS = [
  ['extension dependencies', ['install']],
  ['extension build', ['run', 'build']],
  ['Bridge build', ['run', 'bridge:build']],
];

STEPS.forEach(([label, args], i) => {
  console.log(`\n\x1b[1m▸ ${i + 1}/${STEPS.length}  ${label}\x1b[0m`);
  const r = spawnSync(process.execPath, args, { cwd: root, stdio: 'inherit' });
  if (r.error) {
    console.error(`\n\x1b[31m✗ could not run \`${process.execPath}\`: ${r.error.message}\x1b[0m\n`);
    process.exit(1);
  }
  if (r.status !== 0) {
    // status is null when a signal killed it, so report whichever one actually happened.
    const how = r.signal ? `killed by ${r.signal}` : `exited ${r.status}`;
    console.error(`\n\x1b[31m✗ ${label} failed — \`bun ${args.join(' ')}\` ${how}\x1b[0m\n`);
    process.exit(r.status || 1);
  }
});

console.log(`
\x1b[32m✓ Extension built\x1b[0m  dist/chrome-mv3 — load it unpacked at chrome://extensions, Developer mode on
\x1b[32m✓ Bridge built\x1b[0m     target/release/vexdom (repository root) — the Bridge, the CLI and the MCP server

Two steps are left. Both reach outside this directory, so they stay separate:

  bun run bridge:install   puts \`vexdom\` on your PATH (cargo install, into ~/.cargo/bin)
  vexdom pair              prints a single use code, valid for 10 minutes
`);
