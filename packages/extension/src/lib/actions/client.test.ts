import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { invokeInFrame } from './client';

const NO_RECEIVER = 'Could not establish connection. Receiving end does not exist.';

let sent: unknown[];

beforeEach(() => {
  fakeBrowser.reset();
  sent = [];
  vi.spyOn(fakeBrowser.runtime, 'getManifest').mockReturnValue({
    manifest_version: 3,
    name: 'vexdom',
    version: '0',
    content_scripts: [{ matches: ['<all_urls>'], js: ['content-scripts/content.js'] }],
  });
  // The tab has no content script until one is injected.
  let injected = false;
  vi.spyOn(fakeBrowser.scripting, 'executeScript').mockImplementation(async () => {
    injected = true;
    return [];
  });
  vi.spyOn(fakeBrowser.tabs, 'sendMessage').mockImplementation(async (_tab: number, message: unknown) => {
    sent.push(message);
    if (!injected) throw new Error(NO_RECEIVER);
    return { ok: true, data: { clicked: true } };
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

test('a read that never reached the page is sent again once a content script is injected', async () => {
  const result = await invokeInFrame(5, 0, 'page.extractText', { target: { selector: '#a' } } as never);
  expect(result).toEqual({ ok: true, data: { clicked: true } });
  expect(sent).toHaveLength(2);
});

test('an action that changes the page is not run on a document the site checks never saw', async () => {
  const result = await invokeInFrame(5, 0, 'page.clickElement', { target: { selector: '#a' } } as never);
  expect(result).toMatchObject({ ok: false, error: { code: 'TAB_UNREACHABLE' } });
  expect(sent).toHaveLength(1);
});

test('an action naming a ref is refused as stale: the new content script never issued it', async () => {
  const result = await invokeInFrame(5, 0, 'page.clickElement', { target: { ref: { doc: 'd', id: 1 } } } as never);
  expect(result).toMatchObject({ ok: false, error: { code: 'STALE_REF' } });
  expect(sent).toHaveLength(1);
});
