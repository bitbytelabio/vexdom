// e2e on live, control-heavy sites, anonymously: FireAnt's charts (TradingView's chart library in
// a frame: drawing, clearing drawings, adding and closing a chart pane), TradingView (dialogs, a
// trusted click on the chart canvas), TikTok and YouTube (big trees, paging, search). Opt-in, never
// in CI: it needs the network, the sites change their markup and may block automated browsers. A
// site that cannot be reached, or shows a wall in place of its page, is skipped with the reason.
//
//   bun live.mjs                    # every site
//   bun live.mjs fireant youtube    # some
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT, refOf, start } from './harness.mjs';

// tradingview.com gates drawing tools and multi-chart layouts behind an account ("Join for free
// to access trendline and every other drawing tool"); FireAnt's charts do not.
const FIREANT = 'https://fireant.vn/b/charts';
const TRADINGVIEW = 'https://www.tradingview.com/chart/?symbol=BINANCE%3ABTCUSDT';
const SETTLE_MS = 7000;
const MAX_PAGES = 30;

const wanted = process.argv.slice(2);
const results = [];
const timings = [];

// Agents in these tests act without anyone to approve a form submission.
const env = await start({ config: { requireApproval: [] } });
const { mcp } = env;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const snapshot = (args = {}) => mcp.call('snapshot', args);

async function act(args) {
  const reply = await mcp.call('act', args);
  timings.push({ site: current, what: `act ${args.do}`, ms: reply.ms });
  return reply;
}

class Skip extends Error {}
let current = '';

async function check(name, run) {
  try {
    await run();
    results.push({ name, status: 'ok' });
    console.log(`ok ${name}`);
  } catch (error) {
    const skipped = error instanceof Skip;
    const message = error instanceof Error ? error.message : String(error);
    results.push({ name, status: skipped ? 'skip' : 'FAIL', message });
    console.log(`${skipped ? 'skip' : 'FAIL'} ${name}: ${message.slice(0, 1500)}`);
  }
}

async function open(url) {
  const page = await env.browser.newPage();
  try {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  } catch (error) {
    await page.close();
    throw new Skip(`${url} did not load: ${error.message}`);
  }
  await page.bringToFront();
  await sleep(SETTLE_MS);
  return page;
}

/** A full tree, then every page after it; asserts the pages join without a ref twice. */
async function wholeTree(args = {}) {
  const first = await snapshot({ ...args, full: true });
  const timing = { site: current, what: 'snapshot full', ms: first.ms };
  timings.push(timing);
  assert.ok(!first.isError, first.text);
  const pages = [first.text];
  for (;;) {
    const next = /Read on with snapshot offset:(\d+)/.exec(pages.at(-1));
    if (!next) break;
    assert.ok(pages.length < MAX_PAGES, `more than ${MAX_PAGES} pages`);
    const page = await snapshot({ offset: Number(next[1]) });
    assert.ok(!page.isError, page.text);
    pages.push(page.text);
  }
  const refs = pages.flatMap((text) => [...text.matchAll(/ref=e(\d+)/g)].map((match) => Number(match[1])));
  assert.equal(new Set(refs).size, refs.length, 'a ref shows up on two pages');
  assert.doesNotMatch(pages.at(-1), /\[truncated:/);
  timing.what = `snapshot full (${refs.length} refs, ${pages.length} page${pages.length > 1 ? 's' : ''})`;
  return { text: pages.join('\n'), pages: pages.length, refs: refs.length };
}

/** The pointer events the page saw, recorded in the capture phase (the test's own browser). */
async function recordPointer(page) {
  await page.evaluate(() => {
    window.__vexdomPointer = [];
    addEventListener(
      'pointerdown',
      (event) =>
        window.__vexdomPointer.push({
          x: event.clientX,
          y: event.clientY,
          tag: event.target.tagName,
          trusted: event.isTrusted,
        }),
      { capture: true },
    );
  });
}

/** The chart canvas of FireAnt's chart frame, in the top viewport's CSS px. */
async function chartCanvas(page) {
  const frame = await page.$('iframe');
  const offset = await frame.boundingBox();
  const canvas = await (
    await frame.contentFrame()
  ).evaluate(() => {
    const [box] = [...document.querySelectorAll('canvas')]
      .map((element) => element.getBoundingClientRect())
      .sort((a, b) => b.width * b.height - a.width * a.height);
    return { x: box.x, y: box.y, w: box.width, h: box.height };
  });
  const box = { x: offset.x + canvas.x, y: offset.y + canvas.y, w: canvas.w, h: canvas.h };
  return (fx, fy) => ({ x: Math.round(box.x + box.w * fx), y: Math.round(box.y + box.h * fy) });
}

/** The drawing count FireAnt's "Xóa N bản vẽ" (remove N drawings) button shows, and its ref. */
async function drawings() {
  const { text } = await snapshot({ find: 'bản vẽ' });
  const found = /- button "Xóa (\d+) bản vẽ" \[ref=(e\d+)/.exec(text);
  assert.ok(found, `no drawing counter in:\n${text}`);
  return { count: Number(found[1]), ref: found[2] };
}

const countOf = (text, pattern) => text.match(new RegExp(pattern, 'g'))?.length ?? 0;

const sites = {
  async fireant() {
    const page = await open(FIREANT);
    // The chart frame loads its toolbar after the page: wait for its drawing tools.
    for (let waited = 0; waited < 20_000; waited += 500) {
      if (/button "Đường Xu hướng"/.test((await snapshot({ find: 'Đường Xu hướng' })).text)) break;
      await sleep(500);
    }
    let tree;
    await check('fireant tree', async () => {
      tree = await wholeTree();
      const later = /- button "Để sau" \[ref=(e\d+)/.exec(tree.text);
      if (later) assert.ok(!(await act({ do: 'clickElement', target: later[1] })).isError);
      assert.match(tree.text, /Iframe "Financial Chart"/, 'the chart frame is walked');
      const inFrame = tree.text.split('Iframe "Financial Chart"')[1].split('\n- ')[0];
      assert.ok(countOf(inFrame, 'ref=e\\d+') >= 40, 'the chart frame shows its toolbar');
      const { count, ref } = await drawings();
      if (count > 0) await act({ do: 'clickElement', target: ref });
    });
    if (!tree) return page.close();

    const at = await chartCanvas(page);
    const tool = async (name) => {
      const { text } = await snapshot({ find: name });
      const reply = await act({ do: 'clickElement', target: refOf(text, 'button', name) });
      assert.ok(!reply.isError, reply.text);
    };
    /** Draws with `draw`, then asserts the chart holds one drawing more. */
    const adds = async (draw) => {
      const before = (await drawings()).count;
      await draw();
      let count = (await drawings()).count;
      for (let waited = 0; count === before && waited < 3000; waited += 250) {
        await sleep(250);
        count = (await drawings()).count;
      }
      assert.equal(count, before + 1, 'the chart holds no new drawing');
    };
    const ok = (reply) => assert.ok(!reply.isError, reply.text);

    await check('fireant brush stroke', () =>
      adds(async () => {
        await tool('Cọ vẽ');
        // Low on the chart: the top left is the series legend, which a press would grab.
        ok(await act({ do: 'dragElement', fromPoint: at(0.6, 0.9), toPoint: at(0.8, 0.85), trusted: true, steps: 24 }));
      }),
    );

    await check('fireant text', () =>
      adds(async () => {
        // A click on the chart opens the text editor; type, then confirm.
        await tool('Văn bản');
        ok(await act({ do: 'trustedClick', point: at(0.75, 0.2) }));
        let editor;
        for (let waited = 0; waited < 5000; waited += 250) {
          editor = (await snapshot({ full: true })).text;
          if (/hint="name:text-input"/.test(editor)) break;
          await sleep(250);
        }
        const field = /- textbox \[ref=(e\d+)\] hint="name:text-input"/.exec(editor);
        assert.ok(field, `no text editor in:\n${editor}`);
        ok(await act({ do: 'fillInput', target: field[1], value: 'Vexdom e2e' }));
        ok(await act({ do: 'clickElement', target: refOf(editor, 'button', 'Ok') }));
      }),
    );

    // Line tools are placed as a mouse user places them: click, move, click. (A drag leaves the
    // line waiting for its second click, which the next press anywhere then gives it.)
    await check('fireant trendline from points read off a screenshot', () =>
      adds(async () => {
        const shot = await snapshot({ as: 'image' });
        const scale = Number(/Captured (\d+)×/.exec(shot.text)?.[1]) / (await page.evaluate(() => innerWidth));
        const onShot = ({ x, y }) => ({ x: Math.round(x * scale), y: Math.round(y * scale), source: 'screenshot' });
        await tool('Đường Xu hướng');
        ok(await act({ do: 'trustedClick', point: onShot(at(0.2, 0.65)) }));
        ok(await act({ do: 'trustedClick', point: onShot(at(0.45, 0.35)) }));
      }),
    );

    await check('fireant Fibonacci retracement', () =>
      adds(async () => {
        await tool('Fibonacci thoái lui');
        ok(await act({ do: 'trustedClick', point: at(0.5, 0.75) }));
        ok(await act({ do: 'trustedClick', point: at(0.7, 0.3) }));
      }),
    );

    await check('fireant remove every drawing', async () => {
      const { count, ref } = await drawings();
      assert.ok(count > 0, 'nothing to remove');
      const removed = await act({ do: 'clickElement', target: ref });
      assert.ok(!removed.isError, removed.text);
      assert.equal((await drawings()).count, 0);
    });

    await check('fireant close a chart tab', async () => {
      // Anonymously FireAnt keeps at most 3 charts open; the page opens on 3 tabs.
      const tabs = async () => [
        ...(await snapshot({ full: true })).text.matchAll(/- tab "([^"]+ \(1D\))" \[(?:selected, )?ref=(e\d+)/g),
      ];
      const before = await tabs();
      assert.ok(before.length >= 2, `${before.length} chart tabs`);
      const [, name, ref] = before.find((tab) => !tab[0].includes('selected'));
      const menu = await act({ do: 'clickElement', target: ref, button: 'right' });
      ok(menu);
      ok(await act({ do: 'clickElement', target: refOf(menu.text, 'menuitem', 'Đóng[^"]*') }));
      const after = await tabs();
      assert.equal(after.length, before.length - 1);
      assert.ok(!after.some((tab) => tab[1] === name), `${name} is still open`);
    });

    await check('fireant add a chart pane, then close it', async () => {
      const panes = async () => countOf((await wholeTree()).text, 'Iframe "Financial Chart"');
      assert.equal(await panes(), 1);
      let { text } = await snapshot({ full: true });
      const menu = await act({ do: 'clickElement', target: refOf(text, 'button', 'Bố cục, đồng bộ mã, tách khung…') });
      assert.ok(!menu.isError, menu.text);
      const split = await act({ do: 'clickElement', target: refOf(menu.text, 'menuitem', 'Chia thêm một khung[^"]*') });
      assert.ok(!split.isError, split.text);
      // The new pane opens on a chooser: pick the technical chart.
      let chart;
      for (let waited = 0; !chart && waited < 10_000; waited += 500) {
        await sleep(500);
        chart = /- button "(Biểu đồ kỹ thuật[^"]*)" \[ref=(e\d+)/.exec(
          (await snapshot({ find: 'Biểu đồ kỹ thuật' })).text,
        );
      }
      assert.ok(chart, 'the new pane offers no chart');
      const picked = await act({ do: 'clickElement', target: chart[2] });
      ok(picked);
      for (let waited = 0; (await panes()) < 2 && waited < 15_000; waited += 1000) await sleep(1000);
      assert.equal(await panes(), 2, `the second chart did not open:\n${picked.text}`);

      ({ text } = await snapshot({ full: true }));
      const closers = [...text.matchAll(/- button "Đóng khung" \[ref=(e\d+)/g)].map((match) => match[1]);
      assert.ok(closers.length >= 2, `${closers.length} pane close buttons`);
      const closed = await act({ do: 'clickElement', target: closers.at(-1) });
      assert.ok(!closed.isError, closed.text);
      await sleep(1500);
      assert.equal(await panes(), 1, 'the second chart is still open');
    });
    await page.close();
  },

  async tradingview() {
    const page = await open(TRADINGVIEW);
    let tree;
    await check('tradingview tree', async () => {
      tree = await wholeTree();
      assert.ok(tree.refs >= 60, `${tree.refs} refs`);
      assert.match(tree.text, /region "Chart #1"/);
    });
    if (!tree) return page.close();

    await check('tradingview indicators dialog', async () => {
      const opened = await act({
        do: 'clickElement',
        target: refOf(tree.text, 'button', 'Indicators, metrics, and strategies'),
      });
      assert.ok(!opened.isError, opened.text);
      assert.match(opened.text, /searchbox "Search"/, 'the dialog shows in the delta');
      const search = refOf(opened.text, 'searchbox', 'Search');
      const typed = await act({ do: 'fillInput', target: search, value: 'Relative Strength' });
      assert.ok(!typed.isError, typed.text);
      assert.match(typed.text, /Relative Strength/);
      const closed = await act({ do: 'pressKey', key: 'Escape' });
      assert.ok(!closed.isError, closed.text);
      assert.match(closed.text, /removed/, 'the dialog leaves the delta');
    });

    await check('tradingview interval', async () => {
      const { text } = await wholeTree();
      const reply = await act({ do: 'clickElement', target: refOf(text, 'button', '5 days in 5 minutes intervals') });
      assert.ok(!reply.isError, reply.text);
      assert.match(reply.text, /button "5 minutes"/);
    });

    await check('tradingview canvas point', async () => {
      const shot = await snapshot({ as: 'image' });
      const size = /Captured (\d+)×(\d+)/.exec(shot.text);
      assert.ok(shot.image && size, shot.text);
      const width = Number(size[1]);
      assert.ok(width <= 1024 && Number(size[2]) <= 1024, `captured ${size[1]}×${size[2]}`);
      const scale = width / (await page.evaluate(() => innerWidth));
      const target = await page.evaluate(() => {
        const [canvas] = [...document.querySelectorAll('canvas')]
          .map((element) => ({ element, box: element.getBoundingClientRect() }))
          .sort((a, b) => b.box.width * b.box.height - a.box.width * a.box.height);
        return { x: canvas.box.x + canvas.box.width / 2, y: canvas.box.y + canvas.box.height / 2 };
      });
      await recordPointer(page);
      const click = await act({
        do: 'trustedClick',
        point: { x: Math.round(target.x * scale), y: Math.round(target.y * scale), source: 'screenshot' },
      });
      assert.ok(!click.isError, click.text);
      const [hit] = await page.evaluate(() => window.__vexdomPointer);
      assert.ok(hit, 'no pointer event reached the page');
      assert.ok(hit.trusted, 'the event was synthetic');
      assert.equal(hit.tag, 'CANVAS');
      assert.ok(
        Math.abs(hit.x - target.x) <= 3 && Math.abs(hit.y - target.y) <= 3,
        `hit ${hit.x},${hit.y}, aimed at ${target.x},${target.y} (scale ${scale})`,
      );
    });
    await page.close();
  },

  async tiktok() {
    const page = await open('https://www.tiktok.com/explore');
    let tree;
    await check('tiktok tree', async () => {
      tree = await wholeTree();
      if (/captcha|verify to continue/i.test(tree.text) && tree.refs < 20)
        throw new Skip('TikTok showed a verification wall');
      assert.ok(tree.refs >= 60, `${tree.refs} refs`);
      assert.match(tree.text, /searchbox "Search"/);
    });
    if (!tree) return page.close();

    await check('tiktok category', async () => {
      // The chip's button sits under an overlay of the chip: the clickable chip is the target.
      const reply = await act({ do: 'clickElement', target: refOf(tree.text, 'generic', 'Comedy') });
      assert.ok(!reply.isError, reply.text);
    });

    await check('tiktok scroll and page', async () => {
      // The feed is virtualized, so the tree need not grow: what scrolls is the proof.
      const scrolledBy = () =>
        page.evaluate(() =>
          Math.max(scrollY, ...[...document.querySelectorAll('*')].map((element) => element.scrollTop)),
        );
      const before = await scrolledBy();
      for (let step = 0; step < 4; step++) {
        const scrolled = await act({ do: 'scrollTo', direction: 'down' });
        assert.ok(!scrolled.isError, scrolled.text);
        await sleep(1500);
      }
      assert.ok((await scrolledBy()) > before, 'nothing scrolled');
      const after = await wholeTree();
      assert.ok(after.refs >= 60, `${after.refs} refs after scrolling`);
    });
    await page.close();
  },

  async youtube() {
    const page = await open('https://www.youtube.com/');
    await check('youtube search', async () => {
      const { text } = await wholeTree();
      if (/Before you continue/.test(text)) throw new Skip('YouTube showed its consent wall');
      const typed = await act({ do: 'fillInput', target: refOf(text, 'combobox', 'Search'), value: 'lofi hip hop' });
      assert.ok(!typed.isError, typed.text);
      assert.match(typed.text, /option "lofi hip hop/, 'suggestions show in the delta');
      const sent = await act({ do: 'pressKey', key: 'Enter' });
      assert.ok(!sent.isError, sent.text);
      for (let waited = 0; !page.url().includes('/results') && waited < 10_000; waited += 250) await sleep(250);
      assert.match(page.url(), /\/results\?search_query=lofi/);
      await sleep(3000);
      const results = await wholeTree();
      assert.ok(results.refs >= 30, `${results.refs} refs on the results page`);
      // With link targets the results outgrow one answer: read on page by page.
      await act({ do: 'scrollTo', direction: 'bottom' });
      await sleep(3000);
      const linked = await wholeTree({ urls: true });
      assert.ok(linked.pages >= 2, `${linked.pages} page(s): the tree was not long enough to page`);
      assert.match(linked.text, /\/watch\?v=/);
    });
    await page.close();
  },
};

try {
  for (const [name, run] of Object.entries(sites)) {
    if (wanted.length && !wanted.includes(name)) continue;
    current = name;
    try {
      await run();
    } catch (error) {
      const skipped = error instanceof Skip;
      results.push({ name, status: skipped ? 'skip' : 'FAIL', message: error.message });
      console.log(`${skipped ? 'skip' : 'FAIL'} ${name}: ${error.message}`);
    }
  }
  console.log('\n| site | what | ms |\n|---|---|---|');
  for (const { site, what, ms } of timings) console.log(`| ${site} | ${what} | ${ms.toFixed(0)} |`);
  const out = join(ROOT, 'e2e/out');
  mkdirSync(out, { recursive: true });
  writeFileSync(
    join(out, 'live.json'),
    `${JSON.stringify({ chrome: await env.browser.version(), at: new Date().toISOString(), results, timings }, null, 2)}\n`,
  );
} finally {
  await env.stop();
}

process.exit(results.some(({ status }) => status === 'FAIL') ? 1 : 0);
