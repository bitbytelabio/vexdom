import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { BLOCKED_SITES_KEY, SITE_BLOCKED } from '@/lib/settings/blocked-sites';
import { refusalFor } from '@/lib/bridge/site-guard';
import { exposeActions } from './host';
import { ACTION_CHANNEL, type ActionResult } from './protocol';

type Listener = (message: unknown, sender: unknown, reply: (result: ActionResult) => void) => unknown;

let listener: Listener;

function ask(action: string): Promise<ActionResult> {
  return new Promise((resolve) => listener({ channel: ACTION_CHANNEL, action, input: {} }, {}, resolve));
}

beforeEach(async () => {
  fakeBrowser.reset();
  await fakeBrowser.storage.local.set({ [BLOCKED_SITES_KEY]: ['mybank.com'] });
  vi.spyOn(fakeBrowser.runtime.onMessage, 'addListener').mockImplementation((added) => {
    listener = added as Listener;
  });
  // The background's answer to `siteRefusal` (background.ts), where the list is matched.
  vi.spyOn(fakeBrowser.runtime, 'sendMessage').mockImplementation(async (message: unknown) => refusalFor((message as { url: string }).url));
  exposeActions();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test('a page on a blocked site refuses on its own, whatever the background decided', async () => {
  vi.stubGlobal('location', new URL('https://secure.mybank.com/accounts'));
  expect(await ask('page.extractText')).toMatchObject({ ok: false, error: { code: SITE_BLOCKED } });
});

test('any other page hands the action on as before', async () => {
  vi.stubGlobal('location', new URL('https://example.com/'));
  expect(await ask('page.noSuchAction')).toMatchObject({ ok: false, error: { code: 'UNKNOWN_ACTION' } });
});
