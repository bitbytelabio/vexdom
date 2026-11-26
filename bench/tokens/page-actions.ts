// Bundled by capture.mjs into the page under test: the extension's own page actions, run the way
// the content script runs them.
import { extractText } from '../../packages/extension/src/lib/actions/page/extract-text';
import { getPageInfo } from '../../packages/extension/src/lib/actions/page/get-page-info';
import { readHtml } from '../../packages/extension/src/lib/actions/page/read-html';
import { snapshot } from '../../packages/extension/src/lib/actions/page/snapshot';

(globalThis as unknown as { vexdomBench: unknown }).vexdomBench = {
  getPageInfo: (input?: unknown) => getPageInfo.execute(getPageInfo.input.parse(input ?? {})),
  extractText: async (input?: unknown) => extractText.execute(extractText.input.parse(input ?? {})),
  snapshot: (input?: unknown) => snapshot.execute(snapshot.input.parse(input ?? {})),
  readHtml: (input?: unknown) => readHtml.execute(readHtml.input.parse(input ?? {})),
};
