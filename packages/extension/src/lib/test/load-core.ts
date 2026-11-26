// ext-core's pure half is synchronous once loaded (src/lib/bridge/pure.ts). Tests have no extension
// URL to fetch it from, so every import of the module (a `vi.resetModules()` makes a fresh one)
// loads the built wasm from disk first. `bun run ext-core:build` writes it.
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { vi } from 'vitest';

vi.mock('@/lib/bridge/pure', async (importOriginal) => {
  const pure = await importOriginal<typeof import('@/lib/bridge/pure')>();
  const bytes = await readFile(resolve(process.cwd(), 'src/extension/public/ext-core.wasm'));
  const loadCore = () => pure.loadCoreFrom(async () => bytes);
  await loadCore();
  return { ...pure, loadCore };
});
