// The e2e harness: the Bridge (`vexdom`) in a throwaway VEXDOM_HOME, a headful Chrome for Testing
// with the built extension, paired by code, and `vexdom mcp` driven over stdio JSON-RPC. The
// launch, pair and teardown follow bench/tokens/live.mjs.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import puppeteer from 'puppeteer';

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
export const ROOT = here('../');
const FIXTURES = here('./fixtures/');
export const FIXTURE_PORT = 18800;
export const ORIGIN = `http://127.0.0.1:${FIXTURE_PORT}`;
const LONG_ITEMS = 1500;

function serveFixtures() {
  const long = `<!doctype html><title>Long</title>${Array.from(
    { length: LONG_ITEMS },
    (_, i) => `<a href="/item/${i}">Item ${i}</a><br>`,
  ).join('')}`;
  const server = createServer((req, res) => {
    const name = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '');
    if (name === 'long.html') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(long);
      return;
    }
    if (!/^[\w.-]+\.html$/.test(name)) {
      res.writeHead(404).end();
      return;
    }
    try {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(readFileSync(join(FIXTURES, name)));
    } catch {
      res.writeHead(404).end();
    }
  });
  return new Promise((resolve) => server.listen(FIXTURE_PORT, '127.0.0.1', () => resolve(server)));
}

async function pairBrowser(browser, code) {
  const worker = await browser.waitForTarget(
    (target) => target.type() === 'service_worker' && target.url().startsWith('chrome-extension://'),
    { timeout: 30_000 },
  );
  const id = new URL(worker.url()).host;
  const page = await browser.newPage();
  await page.goto(`chrome-extension://${id}/options.html`);
  // The side panel's code form sends exactly this message (pairing-status.tsx CodeForm).
  const result = await page.evaluate(
    (token) => chrome.runtime.sendMessage({ channel: 'vexdom/bridge', op: 'pair', token }),
    code,
  );
  await page.close();
  if (!result?.ok) throw new Error(`pairing failed: ${JSON.stringify(result)}`);
}

/** `vexdom mcp` over newline-delimited JSON-RPC on stdin/stdout. */
function mcpClient(bin, env) {
  const child = spawn(bin, ['mcp'], { env, stdio: ['pipe', 'pipe', 'inherit'] });
  const waiting = new Map();
  let nextId = 1;
  createInterface({ input: child.stdout }).on('line', (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      return;
    }
    const settle = waiting.get(message.id);
    if (!settle) return;
    waiting.delete(message.id);
    settle(message);
  });
  child.on('exit', (code) => {
    for (const settle of waiting.values()) settle({ error: { message: `vexdom mcp exited (${code})` } });
    waiting.clear();
  });
  const send = (message) => child.stdin.write(`${JSON.stringify(message)}\n`);
  const request = (method, params) =>
    new Promise((resolve) => {
      const id = nextId++;
      waiting.set(id, resolve);
      send({ jsonrpc: '2.0', id, method, params });
    });
  return {
    child,
    request,
    notify: (method) => send({ jsonrpc: '2.0', method }),
    /** One tool call: its text blocks joined, its first image, whether it failed, and its round trip in ms. */
    async call(name, args = {}) {
      const started = performance.now();
      const reply = await request('tools/call', { name, arguments: args });
      const ms = performance.now() - started;
      if (reply.error) throw new Error(`${name}: ${reply.error.message}`);
      const content = reply.result?.content ?? [];
      const text = content
        .filter((block) => block.type === 'text')
        .map((block) => block.text)
        .join('\n');
      const image = content.find((block) => block.type === 'image');
      return {
        text,
        image: image ? { data: image.data, mimeType: image.mimeType } : undefined,
        isError: reply.result?.isError === true,
        ms,
      };
    },
  };
}

/** Chrome for Testing with the built extension, in a fresh profile. */
function launch() {
  const ext = join(ROOT, 'packages/extension/dist/chrome-mv3');
  if (!existsSync(ext)) throw new Error('build the extension first: cd packages/extension && bun run build');
  return puppeteer.launch({
    headless: false,
    enableExtensions: [ext],
    defaultViewport: null,
    args: [
      '--window-size=1280,900',
      '--use-mock-keychain',
      '--password-store=basic',
      ...(process.env.CI ? ['--no-sandbox'] : []),
    ],
  });
}

/** `config` is written to the throwaway home's config.json before the Bridge starts. */
export async function start({ config } = {}) {
  const bin = process.env.VEXDOM_BIN ?? join(ROOT, 'target/release/vexdom');
  if (!existsSync(bin)) throw new Error('build the Bridge first: cargo build --release -p vexdom (or set VEXDOM_BIN)');
  const home = mkdtempSync(join(tmpdir(), 'vexdom-e2e-'));
  const env = { ...process.env, VEXDOM_HOME: home };
  const cli = (...args) => {
    const out = spawnSync(bin, args, { cwd: ROOT, env, encoding: 'utf8', timeout: 60_000 });
    if (out.status !== 0)
      throw new Error(`vexdom ${args.join(' ')} failed (${out.status}): ${out.stderr}${out.stdout}`);
    return out.stdout;
  };

  // `vexdom stop` takes a Bridge too old to report its home on `/health` (the user's installed
  // one, say) for its own, so only the one this run started (its pid in this home's lockfile) is
  // stopped.
  const stopOwnBridge = async () => {
    const { pid } = JSON.parse(readFileSync(join(home, 'daemon.json'), 'utf8'));
    const alive = () => {
      try {
        process.kill(pid, 0);
        return true;
      } catch {
        return false;
      }
    };
    process.kill(pid, 'SIGTERM');
    for (let waited = 0; alive() && waited < 5000; waited += 100)
      await new Promise((resolve) => setTimeout(resolve, 100));
    if (alive()) process.kill(pid, 'SIGKILL');
  };

  let server;
  let browser;
  let mcp;
  const stop = async () => {
    for (const step of [
      () => mcp?.child.kill(),
      () => browser?.close(),
      stopOwnBridge,
      () => server?.close(),
      () => rmSync(home, { recursive: true, force: true }),
    ]) {
      try {
        await step();
      } catch {}
    }
  };

  try {
    if (config) writeFileSync(join(home, 'config.json'), JSON.stringify(config));
    server = await serveFixtures();
    cli('start');
    browser = await launch();
    const code = /Pairing code:\s+([A-Z0-9-]+)/.exec(cli('pair'))?.[1]?.replaceAll('-', '');
    if (!code) throw new Error('no pairing code from `vexdom pair`');
    await pairBrowser(browser, code);

    mcp = mcpClient(bin, env);
    const init = await mcp.request('initialize', {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'vexdom-e2e', version: '0' },
    });
    if (init.error) throw new Error(`initialize: ${init.error.message}`);
    mcp.notify('notifications/initialized');

    const deadline = Date.now() + 15_000;
    for (;;) {
      const { text } = await mcp.call('guide', { topic: 'status' });
      if (/"connected":\s*true/.test(text)) break;
      if (Date.now() > deadline) throw new Error(`the extension never connected: ${text}`);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  } catch (error) {
    await stop();
    throw error;
  }

  return {
    browser,
    mcp,
    stop,
    async page(url) {
      const page = await browser.newPage();
      await page.goto(url, { waitUntil: 'load' });
      await page.bringToFront();
      return page;
    },
  };
}

/** The ref of the `index`-th `- role "name"` line in a snapshot. */
export function refOf(text, role, name, index = 0) {
  const pattern = new RegExp(`- ${role} "${name}"[^\\n]*?ref=(e\\d+)`, 'g');
  const found = [...text.matchAll(pattern)][index]?.[1];
  if (!found) throw new Error(`no ${role} "${name}" #${index} in:\n${text}`);
  return found;
}
