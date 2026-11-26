// Live token bench (P4 plan step 0/13): runs three scripted browser tasks through `claude -p`
// against a vexdom MCP server and records the tokens each task used.
//
//   bun bench/tokens/live.mjs --label before --mcp "node src/daemon/dist/cli.js mcp"   # TS Bridge, gone since P4 (b79f7d8 has it)
//   bun bench/tokens/live.mjs --label after --mcp "target/release/vexdom mcp"
//   bun bench/tokens/live.mjs --report          # live-before.json + live-after.json -> live.md
//
// The CLI is the `--mcp` command minus its trailing `mcp`; the bench uses its `start`, `pair` and
// `stop` commands. Everything runs in a throwaway VEXDOM_HOME, so the user's pairings are never
// touched. A headful Chrome for Testing (puppeteer's) loads dist/chrome-mv3 and pairs with a code
// from `<cli> pair`. Fixture pages come from a sibling ../agent-browser checkout and are
// served on 127.0.0.1:18799.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import puppeteer from 'puppeteer';

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
const root = here('../../');
const fixtures = here('../../../agent-browser/cli/src/native/test_fixtures/');
const FIXTURE_PORT = 18799;
const MODEL = 'sonnet';

const TASKS = [
  {
    id: 'forms',
    url: `http://127.0.0.1:${FIXTURE_PORT}/firefox_forms.html`,
    prompt: 'In the browser tab, type Ada into the Notes field, a@b.c into the Email field and 42 into the Number field, then tell me the value of the Issues field.',
  },
  {
    id: 'diff',
    url: `http://127.0.0.1:${FIXTURE_PORT}/snapshot_diff_probe.html`,
    prompt: 'In the browser tab, type Ada Lovelace into the Account name field, click the Primary action button, then follow the Details link and tell me the page URL.',
  },
  {
    id: 'hn',
    url: 'https://news.ycombinator.com/',
    prompt: 'The browser tab shows Hacker News. Give me the titles of the top 5 stories.',
  },
];

const args = process.argv.slice(2);
const flag = (name) => {
  const at = args.indexOf(name);
  return at === -1 ? undefined : args[at + 1];
};

if (args.includes('--report')) {
  report();
} else {
  const label = flag('--label');
  const mcp = flag('--mcp');
  if (!label || !mcp) {
    console.error('usage: live.mjs --label <name> --mcp "<command> mcp" | --report');
    process.exit(2);
  }
  await run(label, mcp);
}

/** Splits a command line on spaces; the bench's commands have no quoted arguments. */
function words(line) {
  return line.trim().split(/\s+/);
}

function cli(base, env, ...rest) {
  const [command, ...head] = base;
  const out = spawnSync(command, [...head, ...rest], { cwd: root, env, encoding: 'utf8', timeout: 60_000 });
  if (out.status !== 0) throw new Error(`${base.join(' ')} ${rest.join(' ')} failed (${out.status}): ${out.stderr}${out.stdout}`);
  return out.stdout;
}

function serveFixtures() {
  const server = createServer((req, res) => {
    const name = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '');
    if (!/^[\w.-]+\.html$/.test(name)) {
      res.writeHead(404).end();
      return;
    }
    try {
      const body = readFileSync(join(fixtures, name));
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(body);
    } catch {
      res.writeHead(404).end();
    }
  });
  return new Promise((resolve) => server.listen(FIXTURE_PORT, '127.0.0.1', () => resolve(server)));
}

async function pairBrowser(browser, code) {
  const worker = await browser.waitForTarget((target) => target.type() === 'service_worker' && target.url().startsWith('chrome-extension://'), {
    timeout: 30_000,
  });
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

function claude(prompt, configPath, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      'claude',
      [
        '-p',
        prompt,
        '--output-format',
        'json',
        '--model',
        MODEL,
        '--mcp-config',
        configPath,
        '--strict-mcp-config',
        '--allowedTools',
        'mcp__vexdom__*',
        '--tools',
        '',
        '--disable-slash-commands',
        '--no-session-persistence',
        '--setting-sources',
        '',
      ],
      { cwd, stdio: ['ignore', 'pipe', 'pipe'] },
    );
    let out = '';
    let err = '';
    child.stdout.on('data', (chunk) => (out += chunk));
    child.stderr.on('data', (chunk) => (err += chunk));
    child.on('error', reject);
    child.on('close', (status) => {
      try {
        resolve(JSON.parse(out));
      } catch {
        reject(new Error(`claude exited ${status}: ${err}${out}`));
      }
    });
  });
}

async function run(label, mcp) {
  const mcpWords = words(mcp);
  if (mcpWords.at(-1) !== 'mcp') throw new Error('--mcp must end with the `mcp` subcommand');
  const base = mcpWords.slice(0, -1);
  const home = mkdtempSync(join(tmpdir(), 'vexdom-live-'));
  const env = { ...process.env, VEXDOM_HOME: home };
  const configPath = join(home, 'mcp.json');
  // claude runs in the throwaway home, so relative paths become absolute; a bare `node` stays on PATH.
  const absolute = (word) => (word.includes('/') && !word.startsWith('/') ? join(root, word) : word);
  const [command, ...rest] = mcpWords.map(absolute);
  writeFileSync(configPath, JSON.stringify({ mcpServers: { vexdom: { command, args: rest, env: { VEXDOM_HOME: home } } } }));

  const server = await serveFixtures();
  cli(base, env, 'start');
  const browser = await puppeteer.launch({
    headless: false,
    enableExtensions: [join(root, 'packages/extension/dist/chrome-mv3')],
    args: ['--window-size=1280,900'],
    defaultViewport: null,
  });
  const results = [];
  try {
    const code = /Pairing code:\s+([A-Z0-9-]+)/.exec(cli(base, env, 'pair'))?.[1]?.replace('-', '');
    if (!code) throw new Error('no pairing code from `pair`');
    await pairBrowser(browser, code);
    const page = await browser.newPage();
    for (const task of TASKS) {
      await page.goto(task.url, { waitUntil: 'networkidle2', timeout: 60_000 });
      await page.bringToFront();
      const started = Date.now();
      const answer = await claude(task.prompt, configPath, home);
      const usage = answer.usage ?? {};
      const input = (usage.input_tokens ?? 0) + (usage.cache_creation_input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0);
      const row = {
        task: task.id,
        prompt: task.prompt,
        input,
        output: usage.output_tokens ?? 0,
        total: input + (usage.output_tokens ?? 0),
        turns: answer.num_turns,
        costUsd: answer.total_cost_usd,
        ms: Date.now() - started,
        isError: answer.is_error,
        result: answer.result,
      };
      results.push(row);
      console.log(`${task.id}: input ${row.input}, output ${row.output}, turns ${row.turns}`);
    }
  } finally {
    await browser.close();
    server.close();
    try {
      cli(base, env, 'stop');
    } catch (error) {
      console.error(String(error));
    }
    rmSync(home, { recursive: true, force: true });
  }
  writeFileSync(here(`live-${label}.json`), JSON.stringify({ label, mcp, model: MODEL, at: new Date().toISOString(), tasks: results }, null, 2) + '\n');
}

function report() {
  const load = (label) => JSON.parse(readFileSync(here(`live-${label}.json`), 'utf8'));
  const before = load('before');
  const after = load('after');
  const pct = (a, b) => (a === 0 ? 'n/a' : `${(((b - a) / a) * 100).toFixed(1)}%`);
  const lines = [
    '# Live `claude -p` token usage',
    '',
    `Generated by \`bun bench/tokens/live.mjs --report\`; do not edit. Model: ${before.model}.`,
    `Before: \`${before.mcp}\` (${before.at}). After: \`${after.mcp}\` (${after.at}).`,
    '',
    '| Task | Input before | Input after | Output before | Output after | Total before | Total after | Change |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
  ];
  for (const row of before.tasks) {
    const next = after.tasks.find((other) => other.task === row.task);
    if (!next) continue;
    lines.push(`| ${row.task} | ${row.input} | ${next.input} | ${row.output} | ${next.output} | ${row.total} | ${next.total} | ${pct(row.total, next.total)} |`);
  }
  writeFileSync(here('live.md'), lines.join('\n') + '\n');
  console.log(lines.join('\n'));
}
