// Copies what `boltffi pack wasm` wrote for ext-core (into .ext-core/pkg, crates/ext-core/boltffi.toml)
// into the extension: the wasm into public/ (the background loads it by URL) and the JS glue next to
// socket.ts. All three are git-ignored. `bun run ext-core:build` runs it after the pack.
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pkg = join(root, '.ext-core/pkg');
const glue = join(root, 'src/lib/bridge/ext-core');

mkdirSync(glue, { recursive: true });
for (const file of ['ext_core.js', 'ext_core.js.map', 'ext_core.d.ts', 'ext_core_imports.js', 'ext_core_imports.js.map', 'ext_core_imports.d.ts']) {
  copyFileSync(join(pkg, file), join(glue, file));
}
copyFileSync(join(pkg, 'ext_core_bg.wasm'), join(root, 'src/extension/public/ext-core.wasm'));
console.log(`ext-core: glue in ${glue}, wasm in src/extension/public/ext-core.wasm`);
