// Writes the tool manifest the Rust Bridge embeds (ADR-025): crates/vexdom-core/assets/tools.<target>.json at the repository root.
// Run after changing any action: `bun run tools:emit`. src/lib/actions/registry.test.ts fails on drift.
import { build } from 'esbuild';
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const src = fileURLToPath(new URL('../src', import.meta.url));
const out = fileURLToPath(new URL('../../../crates/vexdom-core/assets/', import.meta.url));
const { outputFiles } = await build({
  entryPoints: [`${src}/lib/actions/emit.ts`],
  bundle: true,
  write: false,
  format: 'esm',
  platform: 'node',
  logLevel: 'warning',
  alias: { '@': src },
  define: { 'import.meta.env.FIREFOX': 'false', 'import.meta.env.CHROME': 'true' },
});
const bundled = `data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`;
const { TOOLS_TARGETS, toolsFileText } = (await import(bundled)) as typeof import('../src/lib/actions/emit');

mkdirSync(out, { recursive: true });
for (const target of TOOLS_TARGETS) {
  writeFileSync(`${out}tools.${target}.json`, toolsFileText(target));
  console.log(`wrote tools.${target}.json`);
}
