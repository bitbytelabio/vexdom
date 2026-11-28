// e2e on a real Chromium: the after-action settle, shadow and re-rendered-row refs, screenshot
// points, tree paging, and the latency of act and snapshot. `bun run.mjs` (see harness.mjs for
// what it needs built).
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { ORIGIN, ROOT, refOf, start } from './harness.mjs';

const LONG_ITEMS = 1500;
const MAX_PAGES = 20;
const RUNS = 20;
/** The Bridge's DELTA_BUDGET: an action's reply never waits longer for its delta. */
const BUDGET_MS = 3000;

const env = await start();
const { mcp, browser } = env;
const failed = [];
const series = {};
let page;

async function check(name, run) {
  try {
    await run();
    console.log(`ok ${name}`);
  } catch (error) {
    failed.push(name);
    console.log(`FAIL ${name}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function act(args) {
  return mcp.call('act', args);
}

/** Nearest-rank percentile of sorted samples. */
function percentile(sorted, p) {
  return sorted[Math.max(0, Math.ceil((p / 100) * sorted.length) - 1)];
}

try {
  page = await env.page(`${ORIGIN}/app.html`);

  await check('settle', async () => {
    const { text } = await mcp.call('snapshot', {});
    const reply = await act({ do: 'clickElement', target: refOf(text, 'button', 'Load') });
    series.settle = [reply.ms];
    assert.ok(!reply.isError, reply.text);
    assert.match(reply.text, /button "Loaded 1"/, `the delta did not wait for the late change:\n${reply.text}`);
  });

  await check('shadow', async () => {
    const { text } = await mcp.call('snapshot', { full: true });
    const reply = await act({ do: 'clickElement', target: refOf(text, 'button', 'Shadow buy') });
    assert.ok(!reply.isError, reply.text);
    assert.match(reply.text, /button "Bought"/);
  });

  await check('nth-anchor', async () => {
    const { text } = await mcp.call('snapshot', { full: true });
    const save = refOf(text, 'button', 'Save', 1);
    const rerender = refOf(text, 'button', 'Rerender rows');
    const reply = await act({
      batch: [
        { do: 'clickElement', target: rerender },
        { do: 'clickElement', target: save },
      ],
      bail: true,
    });
    assert.ok(!reply.isError, reply.text);
    assert.doesNotMatch(reply.text, /STALE_REF/);
    assert.match(reply.text, /button "Saved Beta"/);
  });

  await check('screenshot-points', async () => {
    const shot = await mcp.call('snapshot', { as: 'image' });
    assert.ok(shot.image, `no image: ${shot.text}`);
    const size = /Captured (\d+)×(\d+)/.exec(shot.text);
    assert.ok(size, shot.text);
    const [width, height] = [Number(size[1]), Number(size[2])];
    assert.ok(width <= 1024 && height <= 1024, `captured ${width}×${height}`);
    const inner = await page.evaluate(() => innerWidth);
    const scale = width / inner;
    const { cx, cy } = await page.evaluate(() => {
      const box = document.getElementById('pad').getBoundingClientRect();
      return { cx: box.left + box.width / 2, cy: box.top + box.height / 2 };
    });
    const click = await act({
      do: 'trustedClick',
      point: { x: Math.round(cx * scale), y: Math.round(cy * scale), source: 'screenshot' },
    });
    assert.ok(!click.isError, click.text);
    const { text } = await mcp.call('snapshot', {});
    const pad = /button "Pad (\d+),(\d+)"/.exec(text);
    assert.ok(pad, text);
    const [x, y] = [Number(pad[1]), Number(pad[2])];
    assert.ok(
      Math.abs(x - cx) <= 3 && Math.abs(y - cy) <= 3,
      `clicked ${x},${y}, aimed at ${cx},${cy} (scale ${scale})`,
    );
  });

  await check('tree-offset', async () => {
    const long = await env.page(`${ORIGIN}/long.html`);
    try {
      const pages = [(await mcp.call('snapshot', { full: true })).text];
      for (;;) {
        const next = /Read on with snapshot offset:(\d+)/.exec(pages.at(-1));
        if (!next) break;
        assert.ok(pages.length < MAX_PAGES, `more than ${MAX_PAGES} pages`);
        pages.push((await mcp.call('snapshot', { offset: Number(next[1]) })).text);
      }
      assert.ok(pages.length >= 3, `${pages.length} pages`);
      assert.doesNotMatch(pages.at(-1), /truncated/);
      const bodies = pages
        .map((text) =>
          text
            .split('\n')
            .filter((line) => !line.startsWith('<<<') && !line.startsWith('[truncated:'))
            .join('\n'),
        )
        .join('\n');
      const seen = new Map();
      for (const [, i] of bodies.matchAll(/link "Item (\d+)"/g)) seen.set(i, (seen.get(i) ?? 0) + 1);
      for (let i = 0; i < LONG_ITEMS; i++)
        assert.equal(seen.get(String(i)), 1, `Item ${i} seen ${seen.get(String(i)) ?? 0} times`);
    } finally {
      await long.close();
      await page.bringToFront();
    }
  });

  await check('latency', async () => {
    // Full: a delta may not name the button. Its ref stays valid while its label flips.
    const { text } = await mcp.call('snapshot', { full: true });
    const toggle = refOf(text, 'button', 'O(?:ff|n)');
    series.act = [];
    for (let run = 0; run < RUNS; run++) {
      const reply = await act({ do: 'clickElement', target: toggle });
      assert.ok(!reply.isError, reply.text);
      series.act.push(reply.ms);
    }
    series.snapshot = [];
    for (let run = 0; run < RUNS; run++) series.snapshot.push((await mcp.call('snapshot', {})).ms);
    const rows = Object.entries(series).map(([name, samples]) => {
      const sorted = [...samples].sort((a, b) => a - b);
      return { name, n: sorted.length, p50: percentile(sorted, 50), p95: percentile(sorted, 95) };
    });
    console.log('\n| series | n | p50 ms | p95 ms |\n|---|---|---|---|');
    for (const { name, n, p50, p95 } of rows) console.log(`| ${name} | ${n} | ${p50.toFixed(0)} | ${p95.toFixed(0)} |`);
    console.log('');
    const out = join(ROOT, 'e2e/out');
    mkdirSync(out, { recursive: true });
    writeFileSync(
      join(out, 'latency.json'),
      `${JSON.stringify(
        {
          bin: process.env.VEXDOM_BIN ?? join(ROOT, 'target/release/vexdom'),
          chrome: await browser.version(),
          series: Object.fromEntries(rows.map(({ name, ...row }) => [name, { ...row, samples: series[name] }])),
        },
        null,
        2,
      )}\n`,
    );
    for (const { name, p95 } of rows) assert.ok(p95 < BUDGET_MS, `${name} p95 ${p95.toFixed(0)} ms`);
  });
} finally {
  await env.stop();
}

process.exit(failed.length ? 1 : 0);
