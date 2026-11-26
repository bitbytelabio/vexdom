// Captures what getPageInfo, extractText and a viewport screenshot return on the token-baseline
// pages (ADR-023): five agent-browser probe fixtures and three real pages, in headless Chrome
// for Testing at 1280x800. Writes bench/tokens/golden/pages/<fixture>/*.json and index.json;
// real pages' HTML goes to bench/tokens/pages/ (not committed).
//
// `--after` (P4, ADR-025) writes golden-after/pages/<fixture>/ instead: getPageInfo and
// extractText again, plus what the Rust Bridge reads (page.snapshot, page.readHtml) from the same
// load, and for snapshot_diff_probe a second snapshot after typing into Account name.
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import puppeteer from 'puppeteer';

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
const after = process.argv.includes('--after');
const golden = here(after ? 'golden-after/pages/' : 'golden/pages/');
const agentBrowser = here('../../../agent-browser/cli/src/native/test_fixtures/');
const PROBE_LICENSE = 'Apache-2.0 (agent-browser f0a9835)';
// The checkout's own path would otherwise land in every probe golden (document.url).
const PORTABLE_FIXTURES = 'file:///agent-browser/cli/src/native/test_fixtures/';

const fixtures = [
  ...['firefox_forms', 'firefox_probe', 'drag_probe', 'webmcp_probe', 'snapshot_diff_probe'].map((name) => ({
    fixture: name,
    url: `file://${agentBrowser}${name}.html`,
    license: PROBE_LICENSE,
  })),
  { fixture: 'wikipedia-rust', url: 'https://en.wikipedia.org/wiki/Rust_(programming_language)', license: 'CC BY-SA 4.0', real: true },
  { fixture: 'hacker-news', url: 'https://news.ycombinator.com/', license: 'news.ycombinator.com content', real: true },
  { fixture: 'docs-rs-tokio', url: 'https://docs.rs/tokio/latest/tokio/', license: 'MIT', real: true },
];

const bundled = await build({
  entryPoints: [here('page-actions.ts')],
  bundle: true,
  format: 'iife',
  platform: 'browser',
  write: false,
  alias: { '@': here('../../packages/extension/src') },
  nodePaths: [here('../../packages/extension/node_modules')],
  define: { 'import.meta.env.FIREFOX': 'false', 'import.meta.env.CHROME': 'true' },
  logLevel: 'warning',
});
const content = bundled.outputFiles[0].text;

const write = (path, value) =>
  writeFileSync(path, (JSON.stringify(value, null, 2) + '\n').replaceAll(`file://${agentBrowser}`, PORTABLE_FIXTURES));

const browser = await puppeteer.launch({ headless: true });
const index = [];
try {
  for (const { fixture, url, license, real } of fixtures) {
    const page = await browser.newPage();
    await page.setViewport({ width: 1280, height: 800, deviceScaleFactor: 1 });
    await page.goto(url, { waitUntil: 'networkidle2', timeout: 60_000 });
    await page.addScriptTag({ content });
    const dir = `${golden}${fixture}/`;
    mkdirSync(dir, { recursive: true });
    write(`${dir}getPageInfo.json`, await page.evaluate(() => globalThis.vexdomBench.getPageInfo({})));
    write(`${dir}extractText.json`, await page.evaluate(() => globalThis.vexdomBench.extractText({})));
    if (after) {
      write(`${dir}snapshot.json`, await page.evaluate(() => globalThis.vexdomBench.snapshot({ interactive: true })));
      write(`${dir}html.json`, await page.evaluate(() => globalThis.vexdomBench.readHtml({})));
      if (fixture === 'snapshot_diff_probe') {
        await page.type('#account-name', 'Ada Lovelace');
        write(`${dir}snapshot-after.json`, await page.evaluate(() => globalThis.vexdomBench.snapshot({ interactive: true })));
      }
    }
    if (fixture === 'firefox_forms' && !after) {
      const base64 = await page.screenshot({ type: 'jpeg', quality: 80, encoding: 'base64' });
      // screenshot.ts answers a plain viewport capture with exactly these keys, in this order.
      write(`${dir}screenshot.json`, { format: 'jpeg', width: 1280, height: 800, dataUrl: `data:image/jpeg;base64,${base64}` });
    }
    if (real && !after) {
      mkdirSync(here('pages/'), { recursive: true });
      writeFileSync(here(`pages/${fixture}.html`), await page.content());
    }
    index.push({ fixture, url, capturedAt: new Date().toISOString(), license });
    await page.close();
    console.log(`captured ${fixture}`);
  }
} finally {
  await browser.close();
}
write(`${golden}index.json`, index);
