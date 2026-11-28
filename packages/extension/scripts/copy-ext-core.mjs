// Copies what `boltffi pack wasm` wrote for ext-core (into .ext-core/pkg, crates/ext-core/boltffi.toml)
// into the extension: the wasm into public/ (the background loads it by URL) and the JS glue next to
// socket.ts. All three are git-ignored. `bun run ext-core:build` runs it after the pack. A wasm
// over MAX_WASM_BYTES fails the build: every install downloads it and every background start loads it.
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** 2,563,777 bytes on 2026-11-27, plus 10%. */
const MAX_WASM_BYTES = 2_821_000;

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pkg = join(root, '.ext-core/pkg');
const glue = join(root, 'src/lib/bridge/ext-core');

mkdirSync(glue, { recursive: true });
for (const file of [
  'ext_core.js',
  'ext_core.js.map',
  'ext_core.d.ts',
  'ext_core_imports.js',
  'ext_core_imports.js.map',
  'ext_core_imports.d.ts',
]) {
  copyFileSync(join(pkg, file), join(glue, file));
}
const wasm = join(root, 'src/extension/public/ext-core.wasm');
copyFileSync(join(pkg, 'ext_core_bg.wasm'), wasm);
console.log(`ext-core: glue in ${glue}, wasm in src/extension/public/ext-core.wasm`);
const size = statSync(wasm).size;
if (size > MAX_WASM_BYTES) {
  console.error(
    `ext-core.wasm is ${size} bytes, over the ${MAX_WASM_BYTES} budget; raise MAX_WASM_BYTES only on purpose`,
  );
  process.exit(1);
}
