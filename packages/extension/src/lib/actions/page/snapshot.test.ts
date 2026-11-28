import { writeFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { jsonText, vaultSeal } from '@/lib/bridge/pure';
import checkout from '../../../../../../crates/vexdom-snapshot/tests/fixtures/checkout.json';
import { clickElement } from './click-element';
import { resolveTarget, targetSchema } from './dom';
import { snapshot, type PageSnapshot, type WireNode } from './snapshot';

const take = (input: Record<string, unknown> = {}) => snapshot.execute(snapshot.input.parse(input)) as PageSnapshot;

function find(s: PageSnapshot, role: string, name?: string): WireNode {
  const node = s.nodes.find((candidate) => candidate.r === role && (name === undefined || candidate.n === name));
  if (!node) throw new Error(`no ${role} ${name ?? ''} in ${JSON.stringify(s.nodes)}`);
  return node;
}

const refTo = (s: PageSnapshot, node: WireNode, extra: Record<string, unknown> = {}) =>
  targetSchema.parse({ ref: { doc: s.docKey, id: node.id, role: node.r, name: node.n ?? '', ...extra } });

function failure(run: () => unknown): { code?: string; message: string } {
  try {
    run();
  } catch (error) {
    const { code, message } = error as { code?: string; message: string };
    return { code, message };
  }
  throw new Error('it did not fail');
}

/** The tree as indented `role "name" = value {states}` lines. */
function outline(s: PageSnapshot): string[] {
  const lines: string[] = [];
  const visit = (index: number, depth: number) => {
    const { r, n, v, s: states, c } = s.nodes[index];
    const parts = [r, n === undefined ? '' : JSON.stringify(n), v === undefined ? '' : `= ${JSON.stringify(v)}`];
    lines.push(`${'  '.repeat(depth)}${[...parts, states ? JSON.stringify(states) : ''].filter(Boolean).join(' ')}`);
    for (const child of c ?? []) visit(child, depth + 1);
  };
  visit(0, 0);
  return lines;
}

const STALE = { code: 'STALE_REF', message: 'Element ref is stale. Take a new snapshot.' };

beforeEach(() => {
  // happy-dom has no user-agent default for `visibility`, which the role code reads as hidden.
  document.documentElement.style.visibility = 'visible';
  document.title = 'Checkout';
  document.body.innerHTML = '';
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('the tree', () => {
  test('uses the role names the Bridge keys on, explicit false states and one text node per run', () => {
    document.body.innerHTML = `
      <nav aria-label="Primary"><a href="/">Home</a> <a href="/cart">Cart</a></nav>
      <main>
        <h1>Check   out</h1>
        <p>Hello <span>big</span>
          world<br>again</p>
        <label><input type="checkbox"> Agree</label>
        <select aria-label="Size"><option>S</option><option selected>M</option></select>
        <input type="range" aria-label="Tip" value="3">
        <div role="presentation"><button disabled>Pay <img alt="now"></button></div>
      </main>`;
    expect(outline(take())).toEqual([
      'RootWebArea "Checkout"',
      '  navigation "Primary"',
      '    link "Home"',
      '    link "Cart"',
      '  main',
      '    heading "Check out" {"level":1}',
      '    paragraph',
      '      StaticText "Hello big world again"',
      '    generic',
      '      checkbox "Agree" {"checked":"false"}',
      '      StaticText "Agree"',
      '    combobox "Size" = "M" {"expanded":false}',
      '      option "S"',
      '      option "M" {"selected":true}',
      '    slider "Tip" = 3',
      '    button "Pay now" {"disabled":true}',
      '      StaticText "Pay"',
    ]);
  });

  test('carries link targets and leaves out what is hidden, inert or not text', () => {
    document.body.innerHTML = `
      <a href="/pricing">Pricing</a>
      <div style="display:none"><button>Gone</button></div>
      <div aria-hidden="true"><button>Muted</button></div>
      <div inert><button>Asleep</button></div>
      <div style="visibility:hidden">Ghost <button style="visibility:visible">Back</button></div>
      <script>var secret = 1;</script><style>p {}</style>
      <div data-vexdom-overlay><button>Ours</button></div>`;
    const s = take();
    expect(find(s, 'link', 'Pricing').u).toBe(new URL('/pricing', location.href).href);
    expect(outline(s).slice(1)).toEqual(['  link "Pricing"', '  button "Back"']);
  });

  test('flattens and caps a value, and never sends a password', () => {
    document.body.innerHTML =
      '<textarea aria-label="Note"></textarea><input type="password" aria-label="Password" value="hunter2">';
    document.querySelector('textarea')!.value = `first\n\n  second\tthird ${'😀'.repeat(300)}`;
    const s = take();
    const note = find(s, 'textbox', 'Note').v as string;
    expect(note.startsWith('first second third 😀')).toBe(true);
    expect([...note]).toHaveLength(200);
    expect(note.endsWith('😀')).toBe(true);
    expect(find(s, 'textbox', 'Password')).not.toHaveProperty('v');
  });

  test('sends only well-formed text', () => {
    document.body.innerHTML = '<p></p>';
    document.querySelector('p')!.textContent = 'broken \ud800 pair';
    expect(find(take(), 'StaticText').n).toBe('broken \ufffd pair');
  });

  test('a selector walks only its outermost matches', () => {
    document.body.innerHTML = `
      <nav><a href="/a">Away</a></nav>
      <section class="x"><button>In</button><div class="x"><button>Nested</button></div></section>
      <button>Out</button>`;
    expect(outline(take({ selector: '.x' }))).toEqual([
      'RootWebArea "Checkout"',
      '  generic',
      '    button "In"',
      '    generic',
      '      button "Nested"',
    ]);
    expect(failure(() => take({ selector: '.none' }))).toMatchObject({ code: 'TARGET_NOT_FOUND' });
    expect(failure(() => take({ selector: '[' }))).toMatchObject({ code: 'INVALID_TARGET' });
  });

  test('interactive mode sends every element that can get a ref but table cells, and nothing else', () => {
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 100, 20));
    document.body.innerHTML = `
      <main><h2>Cart</h2><h3></h3>
        <table><tr><th>Item</th><td>Mug</td></tr></table>
        <ul><li>Plain</li><li aria-label="Named">Named</li></ul>
        <div style="cursor:pointer">Quick view</div>
        <form><input aria-label="Coupon"><button>Apply</button></form>
      </main>`;
    const full = take();
    const interactive = take({ interactive: true });
    const ids = (s: PageSnapshot) => s.nodes.flatMap((node) => (node.id === undefined ? [] : [node.id])).sort();
    expect(ids(full)).toHaveLength(7);
    const cellRoles = ['cell', 'gridcell', 'columnheader', 'rowheader'];
    const cells = full.nodes.filter((node) => cellRoles.includes(node.r)).map((node) => node.id);
    expect(cells).toHaveLength(2);
    expect(ids(interactive)).toEqual(ids(full).filter((id) => !cells.includes(id)));
    expect(interactive.nodes.filter((node) => node.id === undefined).map((node) => node.r)).toEqual(['RootWebArea']);
  });

  test('stops at maxNodes and says so', () => {
    document.body.innerHTML = '<button>Go</button>'.repeat(20);
    const s = take({ maxNodes: 5 });
    expect(s.nodes).toHaveLength(5);
    expect(s.stats).toMatchObject({ nodes: 5, truncated: true });
    expect(s.nodes.flatMap((node) => node.c ?? []).every((child) => child > 0 && child < 5)).toBe(true);
  });

  test('settleMs waits for the DOM to go quiet, not for the whole cap', async () => {
    document.body.innerHTML = '<button>First</button>';
    setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<button>Late</button>'), 20);
    const started = performance.now();
    const settled = (await snapshot.execute(snapshot.input.parse({ settleMs: 300 }))) as PageSnapshot;
    expect(performance.now() - started).toBeLessThan(300);
    expect(settled.nodes.filter((node) => node.r === 'button').map((node) => node.n)).toEqual(['First', 'Late']);
  });

  test('viewport mode leaves out what lies wholly off screen and counts it', () => {
    vi.stubGlobal('innerHeight', 800);
    document.body.innerHTML = '<button id="near">Near</button><button id="far">Far</button>';
    const at = (top: number) => () => new DOMRect(0, top, 100, 20);
    vi.spyOn(document.getElementById('near')!, 'getBoundingClientRect').mockImplementation(at(0));
    vi.spyOn(document.getElementById('far')!, 'getBoundingClientRect').mockImplementation(at(5000));
    const s = take({ viewport: true });
    expect(s.nodes.filter((node) => node.r === 'button').map((node) => node.n)).toEqual(['Near']);
    expect(s.offscreen).toEqual({ above: 0, below: 1, aside: 0 });
    expect(take().offscreen).toBeUndefined();
    vi.unstubAllGlobals();
  });
});

describe('the vendored role patches', () => {
  test('a summary is a button carrying the state of its details, whose closed content is left out', () => {
    document.body.innerHTML = `
      <details><summary>More</summary><a href="/x">Hidden link</a></details>
      <details open><summary>Less</summary><p>Shown</p></details>`;
    expect(outline(take()).slice(1)).toEqual([
      '  group',
      '    button "More" {"expanded":false}',
      '  group',
      '    button "Less" {"expanded":true}',
      '    paragraph',
      '      StaticText "Shown"',
    ]);
  });

  test('a closed shadow root is walked, its elements are marked, and a ref into it is found again only inside its host', () => {
    document.body.innerHTML = '<div id="host"><b>light</b></div>';
    const host = document.getElementById('host')!;
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = '<button>Buy</button><slot></slot>';
    // Firefox lends content scripts `openOrClosedShadowRoot`; Chrome, chrome.dom.
    Object.defineProperty(host, 'openOrClosedShadowRoot', { value: root });
    const s = take();
    const buy = find(s, 'button', 'Buy');
    expect(buy.sh).toBe(true);
    expect(outline(s)).toContain('    StaticText "light"');
    expect(resolveTarget(refTo(s, buy))).toBe(root.querySelector('button'));
    root.innerHTML = '<button>Buy</button>';
    expect(resolveTarget(refTo(s, buy))).toBe(root.querySelector('button'));
    host.remove();
    expect(failure(() => resolveTarget(refTo(s, buy)))).toEqual(STALE);
  });
});

describe('element ids', () => {
  test('stay with their element across snapshots, and a new element gets a new one', () => {
    document.body.innerHTML = '<button>One</button><button>Two</button>';
    const ids = (s: PageSnapshot) =>
      Object.fromEntries(s.nodes.filter((node) => node.id).map((node) => [node.n, node.id!]));
    const before = ids(take());
    document.body.insertAdjacentHTML('afterbegin', '<button>Zero</button>');
    document.querySelector('button')!.nextElementSibling!.textContent = 'Uno';
    const after = ids(take());
    expect(after.Uno).toBe(before.One);
    expect(after.Two).toBe(before.Two);
    expect(after.Zero).toBeGreaterThan(Math.max(before.One, before.Two));
  });

  test('go only to elements that can get a ref', () => {
    document.body.innerHTML = '<h1>Title</h1><h2></h2><p>Text</p><button>Go</button>';
    const s = take();
    expect(s.nodes.filter((node) => node.id).map((node) => node.r)).toEqual(['heading', 'button']);
  });

  test('are a UUID-keyed table even where randomUUID is missing', async () => {
    const { crypto: real } = globalThis;
    vi.resetModules();
    vi.stubGlobal('crypto', { getRandomValues: (bytes: Uint8Array<ArrayBuffer>) => real.getRandomValues(bytes) });
    try {
      const fresh = await import('./snapshot');
      expect(fresh.documentKey()).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('a ref', () => {
  beforeEach(() => {
    document.body.innerHTML = '<ul><li><button>Delete</button></li><li><button>Save</button></li></ul>';
  });

  test('names its element while it is in the page', () => {
    const s = take();
    expect(resolveTarget(refTo(s, find(s, 'button', 'Save')))).toBe(document.querySelectorAll('button')[1]);
  });

  test('from a document that is gone is refused', () => {
    const s = take();
    expect(failure(() => resolveTarget(refTo(s, find(s, 'button', 'Save'), { doc: 'another-document' })))).toEqual({
      code: 'STALE_REF',
      message:
        'Ref is from a document that is no longer loaded (the page navigated or the frame was removed). Take a new snapshot and use a fresh ref.',
    });
  });

  test('to a removed element is stale', () => {
    const s = take();
    document.querySelectorAll('li')[1].remove();
    expect(failure(() => resolveTarget(refTo(s, find(s, 'button', 'Save'))))).toEqual(STALE);
  });

  test('to a replaced element finds the one element with its role and name', () => {
    const s = take();
    const item = document.querySelectorAll('li')[1];
    item.innerHTML = '<button>Save</button>';
    expect(resolveTarget(refTo(s, find(s, 'button', 'Save')))).toBe(item.firstElementChild);
  });

  test('that needed nth, or whose role and name are now shared, is not guessed', () => {
    const s = take();
    const item = document.querySelectorAll('li')[1];
    item.innerHTML = '<button>Save</button>';
    expect(failure(() => resolveTarget(refTo(s, find(s, 'button', 'Save'), { nth: 0 })))).toEqual(STALE);
    item.insertAdjacentHTML('beforeend', '<button>Save</button>');
    expect(failure(() => resolveTarget(refTo(s, find(s, 'button', 'Save'))))).toEqual(STALE);
  });

  test('that needed nth is found again in its keyed row after a re-render', () => {
    const row = (key: string) => `<li id="row-${key}">${key.toUpperCase()} <button>Save</button></li>`;
    document.body.innerHTML = `<ul>${row('a')}${row('b')}</ul>`;
    const s = take();
    const second = s.nodes.filter((node) => node.r === 'button' && node.n === 'Save')[1];
    document.querySelector('ul')!.innerHTML = row('b') + row('a');
    expect(resolveTarget(refTo(s, second, { nth: 1 }))).toBe(document.querySelector('#row-b button'));
  });

  test('that needed nth is not guessed when its row is gone or now holds two', () => {
    const markup = '<ul><li>A <button>Save</button></li><li>B <button>Save</button></li></ul>';
    document.body.innerHTML = markup;
    let s = take();
    let second = s.nodes.filter((node) => node.r === 'button' && node.n === 'Save')[1];
    document.querySelectorAll('li')[1].remove();
    expect(failure(() => resolveTarget(refTo(s, second, { nth: 1 })))).toEqual(STALE);

    document.body.innerHTML = markup;
    s = take();
    second = s.nodes.filter((node) => node.r === 'button' && node.n === 'Save')[1];
    const item = document.querySelectorAll('li')[1];
    item.insertAdjacentHTML('beforeend', '<button>Save</button>');
    item.querySelector('button')!.replaceWith(Object.assign(document.createElement('button'), { textContent: 'Save' }));
    expect(failure(() => resolveTarget(refTo(s, second, { nth: 1 })))).toEqual(STALE);
  });

  test('to an SVG control is clicked by event, as it has no click()', async () => {
    document.body.innerHTML = '<svg role="button" aria-label="Zoom"></svg>';
    const s = take();
    const clicks = vi.fn();
    document.querySelector('svg')!.addEventListener('click', clicks);
    await clickElement.execute(
      clickElement.input.parse({ target: refTo(s, find(s, 'button', 'Zoom')), scrollIntoView: false, force: true }),
    );
    expect(clicks).toHaveBeenCalledOnce();
  });

  test('and the key of its document come through sealing whole', () => {
    document.body.innerHTML = '<p>Your temporary password is Xk29-vQ7m!</p><button>password: hunter2x</button>';
    const s = take();
    const target = refTo(s, find(s, 'button'));
    const vault = JSON.stringify({ tag: 'abcdef12', entries: [] });
    const sealed = JSON.parse(vaultSeal(vault, jsonText({ snapshot: s, target }), null, Date.now(), 'seed').value) as {
      snapshot: typeof s;
      target: typeof target;
    };
    expect(sealed.snapshot.docKey).toBe(s.docKey);
    expect(sealed.target).toEqual(target);
    expect(find(sealed.snapshot, 'StaticText').n).not.toContain('Xk29-vQ7m');
  });
});

describe('frames', () => {
  test('a blank same-origin frame is walked inline, under its Iframe node', () => {
    document.body.innerHTML = '<iframe title="Note"></iframe>';
    const frame = document.querySelector('iframe')!.contentDocument!;
    frame.documentElement.style.visibility = 'visible';
    frame.body.innerHTML = '<button>Inside</button>';
    const s = take();
    expect(outline(s)).toEqual(['RootWebArea "Checkout"', '  Iframe "Note"', '    button "Inside"']);
    expect(s.frames).toEqual([]);
    expect(resolveTarget(refTo(s, find(s, 'button', 'Inside')))).toBe(frame.querySelector('button'));
  });

  test('any other frame gets a slot for the extension to fill in', () => {
    document.body.innerHTML = '<iframe title="Pay" name="pay"></iframe>';
    Object.defineProperty(document.querySelector('iframe')!, 'contentDocument', { value: null });
    const s = take();
    // No frame id from the page (Chrome): the slot carries the nonce posted into the frame instead.
    expect(s.frames).toEqual([
      {
        node: s.nodes.indexOf(find(s, 'Iframe', 'Pay')),
        probe: expect.any(String),
        title: 'Pay',
        name: 'pay',
        sameOrigin: false,
      },
    ]);
    expect(take({ includeFrames: false }).frames).toEqual([]);
  });
});

describe('controls without a role', () => {
  test('carry why they act like one; a label around a hidden radio says which', () => {
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 100, 20));
    document.body.innerHTML = `
      <div style="cursor:pointer">Quick <span>view</span></div>
      <label style="cursor:pointer"><input type="radio" style="display:none" checked> Express</label>`;
    const s = take();
    const cursor = {
      hasCursorPointer: true,
      hasOnClick: false,
      hasTabIndex: false,
      isEditable: false,
      isNative: false,
    };
    expect(s.nodes.filter((node) => node.k).map(({ r, k }) => ({ r, k }))).toEqual([
      { r: 'generic', k: { ...cursor, text: 'Quick view', hiddenInputType: null, hiddenInputChecked: null } },
      { r: 'generic', k: { ...cursor, text: 'Express', hiddenInputType: 'radio', hiddenInputChecked: 'true' } },
    ]);
  });

  test('controls that would print alike get a hint; the rest get none', () => {
    document.body.innerHTML = `
      <ul>
        <li>Blue mug <button data-testid="remove-mug">Remove</button></li>
        <li>Red cup <button>Remove</button></li>
      </ul>
      <button><i class="fa fa-trash"></i></button>
      <button>Save</button>`;
    const s = take();
    expect(s.nodes.filter((node) => node.r === 'button').map(({ n, h }) => [n, h])).toEqual([
      ['Remove', 'data-testid:remove-mug; in:Blue mug'],
      ['Remove', 'in:Red cup'],
      [undefined, 'icon:fa-trash'],
      ['Save', undefined],
    ]);
  });

  test('an icon-only control is told by its tooltip and the words or label beside it', () => {
    document.body.innerHTML = `
      <div><div tabindex="0"><span>Brush</span><button><span title="Add to favorites" style="visibility:hidden"><svg></svg></span></button></div></div>
      <div><div tabindex="0"><span>Highlighter</span><button><span title="Add to favorites" style="visibility:hidden"><svg></svg></span></button></div></div>
      <div><div><button aria-label="Magnet mode"><svg></svg></button><button class="arrow"><svg></svg></button></div></div>`;
    const s = take();
    expect(s.nodes.filter((node) => node.r === 'button' && !node.n).map(({ h }) => h)).toEqual([
      'tip:Add to favorites; in:Brush',
      'tip:Add to favorites; in:Highlighter',
      'in:Magnet mode',
    ]);
  });
});

describe('the wire', () => {
  test('has the keys of the Bridge fixture, which the Rust reader parses', () => {
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 100, 20));
    document.body.innerHTML = `
      <nav><a href="/">Home</a></nav>
      <main>
        <h1>Checkout</h1>
        <form aria-label="Order">
          <label>Email <input type="email" required value="a@b.c"></label>
          <label><input type="checkbox" checked> Agree</label>
          <select aria-label="Shipping"><option>Standard</option><option selected>Express</option></select>
          <ul><li>Blue mug <button>Remove</button></li><li>Red cup <button>Remove</button></li></ul>
          <button type="submit">Save</button>
        </form>
        <div style="cursor:pointer">Shipping is free</div>
        <div id="host"></div>
        <iframe title="Pay"></iframe>
      </main>`;
    document.getElementById('host')!.attachShadow({ mode: 'open' }).innerHTML = '<button>Buy</button>';
    Object.defineProperty(document.querySelector('iframe')!, 'contentDocument', { value: null });
    const s = take();
    if (process.env.VEXDOM_SNAPSHOT_DUMP) writeFileSync(process.env.VEXDOM_SNAPSHOT_DUMP, JSON.stringify(s));

    const fixture = checkout as unknown as PageSnapshot;
    expect(Object.keys(s)).toEqual(Object.keys(fixture));
    const keysOf = (objects: object[]) => new Set(objects.flatMap((object) => Object.keys(object)));
    const nodeKeys = new Set([...keysOf(fixture.nodes), 'sh']);
    expect([...keysOf(s.nodes)].filter((key) => !nodeKeys.has(key))).toEqual([]);
    for (const key of ['id', 'r', 'n', 'v', 's', 'c', 'k', 'u', 'h', 'sh']) expect(keysOf(s.nodes)).toContain(key);
    const states = ['level', 'checked', 'expanded', 'selected', 'disabled', 'required', 'focused'];
    expect([...keysOf(s.nodes.flatMap((node) => node.s ?? []))].filter((key) => !states.includes(key))).toEqual([]);
    expect([...keysOf(s.nodes.flatMap((node) => node.k ?? []))]).toEqual(
      Object.keys(fixture.nodes.find((node) => node.k)!.k!),
    );
    expect(Object.keys(s.viewport)).toEqual(Object.keys(fixture.viewport));
    expect(Object.keys(s.stats)).toEqual(Object.keys(fixture.stats));
    const slots = ['node', 'frameId', 'probe', 'title', 'name', 'sameOrigin', 'snapshot'];
    expect(s.frames).toHaveLength(1);
    expect([...keysOf(s.frames)].filter((key) => !slots.includes(key))).toEqual([]);

    // The Rust reader takes `c` as indexes that each name one child, never the root.
    const children = s.nodes.flatMap((node) => node.c ?? []);
    expect(new Set(children).size).toBe(children.length);
    expect(children.every((child) => child > 0 && child < s.nodes.length)).toBe(true);
    expect(
      s.nodes.every(({ id, v }) => (id === undefined || id > 0) && (typeof v !== 'number' || Number.isFinite(v))),
    ).toBe(true);
  });
});
