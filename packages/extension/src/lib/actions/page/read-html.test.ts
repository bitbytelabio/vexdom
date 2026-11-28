import { beforeEach, expect, test } from 'vitest';
import { readHtml } from './read-html';
import { snapshot, type PageSnapshot } from './snapshot';

const read = (input: Record<string, unknown> = {}) =>
  readHtml.execute(readHtml.input.parse(input)) as { url: string; title: string; html: string };

beforeEach(() => {
  document.documentElement.style.visibility = 'visible';
  document.title = 'Shop';
  document.body.innerHTML = `
    <main><h1>Shop</h1><script>boot({ token: 1 })</script><style>p {}</style><!-- a > b -->
      <p>Hello <svg><path d="M0"/></svg>world</p><template><p>Later</p></template><noscript>Enable JS</noscript>
    </main>
    <div data-vexdom-overlay>ring</div>`;
});

test('reads the whole document without what the markdown drops, and leaves the page as it was', () => {
  const { url, title, html } = read();
  expect([url, title]).toEqual([location.href, 'Shop']);
  expect(html.startsWith('<html')).toBe(true);
  expect(html).toContain('<h1>Shop</h1>');
  expect(html).toContain('<p>Hello world</p>');
  for (const dropped of ['boot(', 'p {}', 'a > b', '<svg', 'Later', 'Enable JS', 'ring'])
    expect(html).not.toContain(dropped);
  expect(document.querySelectorAll('script, style, svg, template, noscript')).toHaveLength(5);
});

test('leaves out what the page does not show, where injected instructions hide', () => {
  document.body.innerHTML = `<main><div style="display:none">secret</div><p>shown</p>
    <div style="visibility:hidden">quiet<span style="visibility:visible">back</span></div>
    <a href="/cart">Cart</a></main>`;
  const { html } = read();
  expect(html).toContain('<p>shown</p>');
  expect(html).toContain('back');
  expect(html).toContain(`href="${new URL('/cart', location.href).href}"`);
  for (const hidden of ['secret', 'quiet']) expect(html).not.toContain(hidden);
  expect(document.body.textContent).toContain('secret');
  // A page that hides the root and shows the body again still has its hidden text filtered.
  document.documentElement.style.visibility = 'hidden';
  document.body.style.visibility = 'visible';
  expect(read().html).not.toContain('secret');
  // A hidden element the agent names is read.
  expect(read({ target: { selector: 'main > div' } }).html).toContain('secret');
});

test('reads one element by selector or by ref', () => {
  expect(read({ target: { selector: 'h1' } }).html).toBe('<h1>Shop</h1>');
  const page = snapshot.execute(snapshot.input.parse({})) as PageSnapshot;
  const heading = page.nodes.find((node) => node.r === 'heading')!;
  expect(read({ target: { ref: { doc: page.docKey, id: heading.id } } }).html).toBe('<h1>Shop</h1>');
});
