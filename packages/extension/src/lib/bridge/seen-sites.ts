import { browser } from 'wxt/browser';

/** The site each tab was on when its caller last looked at it. */
const SEEN_KEY = 'vexdom/seenSites';

/**
 * Actions that show the caller what a page is. `page.query` is left out: it reads one fact, and
 * the Bridge asks it for the page's address before acting in Ask and Follow-a-plan modes.
 */
export const LOOKS = new Set([
  'page.snapshot',
  'page.getPageInfo',
  'page.screenshot',
  'page.readHtml',
  'page.extractText',
  'page.pickElement',
]);

/** Actions that change a page, which must not land on a site the caller has not looked at. */
export const PAGE_CHANGERS = new Set([
  'page.clickElement',
  'page.trustedClick',
  'page.fillInput',
  'page.typeText',
  'page.insertText',
  'page.pressKey',
  'page.selectOption',
  'page.setChecked',
  'page.submitForm',
  'page.dragElement',
  'page.mouse',
  'page.attachFile',
  'page.focusInput',
  'page.selectText',
  'page.evaluate',
  'page.runCode',
  'page.injectCode',
  'page.callSiteTool',
  'page.searchSite',
  'page.solveCaptcha',
  'page.dialog',
]);

/** Whoever looked: a conversation's id, or this for every caller outside one (MCP, the CLI). */
export const OUTSIDE_CALLER = 'external';

let queue: Promise<unknown> = Promise.resolve();

const keyOf = (tabId: number, caller: string) => `${caller}:${tabId}`;

async function readSeen(): Promise<Record<string, string>> {
  return ((await browser.storage.session.get(SEEN_KEY))[SEEN_KEY] as Record<string, string> | undefined) ?? {};
}

function change(edit: (seen: Record<string, string>) => void): Promise<void> {
  const run = queue.then(async () => {
    const seen = { ...(await readSeen()) };
    edit(seen);
    await browser.storage.session.set({ [SEEN_KEY]: seen });
  });
  queue = run.catch(() => undefined);
  return run;
}

/**
 * `caller` looked at `tabId` on `host`; with no host (a browser page) there is nothing to hold it to.
 * Each caller is held to what it saw itself: another's look does not stand in for its own.
 */
export function rememberSeen(tabId: number, host: string | undefined, caller: string): Promise<void> {
  return change((seen) => {
    if (host) seen[keyOf(tabId, caller)] = host;
    else delete seen[keyOf(tabId, caller)];
  });
}

export async function seenHost(tabId: number, caller: string): Promise<string | undefined> {
  return (await readSeen())[keyOf(tabId, caller)];
}

/** A closed tab: every caller's look at it goes. */
export function forgetSeen(tabId: number): Promise<void> {
  return change((seen) => {
    for (const key of Object.keys(seen)) if (key.endsWith(`:${tabId}`)) delete seen[key];
  });
}

export function serveSeenSites(): void {
  browser.tabs.onRemoved.addListener((tabId) => void forgetSeen(tabId));
}
