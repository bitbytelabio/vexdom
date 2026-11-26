import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { actThreshold, loadCoreFrom, routeIntent } from '../src/lib/bridge/pure';

const utterance = process.argv.slice(2).join(' ').trim();
if (!utterance) {
  console.error('usage: bun run check:intent "<utterance>"\nThe fixture table runs with: bun run test src/lib/bridge/intent');
  process.exit(1);
}

// The grammar is vexdom-pure's, through ext-core (`bun run ext-core:build` writes the wasm).
await loadCoreFrom(() => readFile(fileURLToPath(new URL('../src/extension/public/ext-core.wasm', import.meta.url))));
console.log(`${utterance}\n${JSON.stringify(routeIntent(utterance), null, 2)}\nacts at ${actThreshold()}`);
