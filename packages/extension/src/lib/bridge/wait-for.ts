import { browser } from 'wxt/browser';
import { z } from 'zod';
import { waitFor } from '@/lib/actions/page/wait-for';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { evaluateInTab } from './evaluate';
import { focusedFrame } from './frame-focus';
import { refusalForTab } from './site-guard';

const POLL_MS = 100;
const NETWORK_IDLE_MS = 500;
const FN_TIMEOUT_MS = 2_000;

/** `**` is any run of characters, `*` any within one path segment; a pattern without `*` is a substring. */
export function urlMatcher(pattern: string): (url: string) => boolean {
  if (!pattern.includes('*')) return (url) => url.includes(pattern);
  const source = pattern
    .split('**')
    .map((part) =>
      part
        .split('*')
        .map((piece) => piece.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&'))
        .join('[^/]*'),
    )
    .join('.*');
  const regex = new RegExp(`^${source}$`);
  return (url) => regex.test(url);
}

async function inFrame<T>(tabId: number, frameId: number, func: (...args: never[]) => T, args: unknown[] = []): Promise<T | undefined> {
  const [injection] = await browser.scripting
    .executeScript({ target: { tabId, frameIds: [frameId] }, func: func as () => T, args: args as [] })
    .catch(() => []);
  return injection?.result as T | undefined;
}

function documentReady(): boolean {
  return document.readyState !== 'loading';
}

/** Milliseconds since the last resource finished, or since the page started when none has. */
function quietFor(): number {
  const ends = performance.getEntriesByType('resource').map((entry) => (entry as PerformanceResourceTiming).responseEnd);
  return performance.now() - (ends.length ? Math.max(...ends) : 0);
}

function pageHasText(text: string): boolean {
  return document.body?.innerText.includes(text) ?? false;
}

const describeConditions = (input: z.output<typeof waitFor.input>): string =>
  [
    input.text !== undefined && `text ${JSON.stringify(input.text)}`,
    input.url !== undefined && `url ${JSON.stringify(input.url)}`,
    input.load !== undefined && `load "${input.load}"`,
    input.fn !== undefined && `fn ${JSON.stringify(input.fn)}`,
  ]
    .filter(Boolean)
    .join(', ');

/** `page.waitFor`: polls every 100 ms until all the given conditions hold. */
export async function waitForInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = waitFor.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { text, url, load, fn, timeoutMs } = parsed.data;
  if (text === undefined && url === undefined && load === undefined && fn === undefined) {
    return failure('INVALID_INPUT', 'Give at least one of "text", "url", "load" or "fn" to wait for.');
  }
  const matchesUrl = url === undefined ? null : urlMatcher(url);
  const started = Date.now();

  for (;;) {
    const tab = await browser.tabs.get(tabId).catch(() => null);
    if (!tab) return failure('TAB_UNREACHABLE', 'The tab closed while waiting.');
    // The tab can move while this polls: nothing is injected into a page on the blocked list.
    const refused = await refusalForTab(tabId);
    if (refused) return refused;
    const frameId = await focusedFrame(tabId);
    let holds = true;
    if (holds && matchesUrl) holds = matchesUrl(tab.url ?? '');
    if (holds && load === 'load') holds = tab.status === 'complete';
    if (holds && load === 'domcontentloaded') holds = (await inFrame(tabId, 0, documentReady)) === true;
    if (holds && load === 'networkidle') {
      holds = tab.status === 'complete' && ((await inFrame(tabId, 0, quietFor)) ?? 0) >= NETWORK_IDLE_MS;
    }
    if (holds && text !== undefined) holds = (await inFrame(tabId, frameId, pageHasText, [text])) === true;
    if (holds && fn !== undefined) {
      const code = `(() => { const v = (${fn}); return typeof v === 'function' ? v() : v; })()`;
      const result = await evaluateInTab(tabId, code, FN_TIMEOUT_MS);
      if (!result.ok) return result;
      holds = !!(result.data as { value?: unknown }).value;
    }
    if (holds) return success({ waitedMs: Date.now() - started, url: tab.url });
    if (Date.now() - started >= timeoutMs) {
      return failure('TIMEOUT', `Waited ${timeoutMs} ms for ${describeConditions(parsed.data)}.`);
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
}
