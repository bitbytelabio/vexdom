import { browser } from 'wxt/browser';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';

type Via = { via: 'next' | 'history' };

/** Runs in the page's own world: Next.js's router when the page has one, else pushState plus popstate. */
function pushRoute(url: string): Via {
  const next = (window as unknown as { next?: { router?: { push?: (url: string) => unknown } } }).next;
  if (typeof next?.router?.push === 'function') {
    void next.router.push(url);
    return { via: 'next' };
  }
  history.pushState({}, '', url);
  window.dispatchEvent(new PopStateEvent('popstate', { state: {} }));
  return { via: 'history' };
}

/** `page.pushState`: a same-origin route change in the top frame, without a reload. */
export async function pushStateInTab(tabId: number, tabUrl: string | undefined, input: unknown): Promise<ActionResult> {
  const url = (input as { url?: unknown } | undefined)?.url;
  if (typeof url !== 'string' || !url) return failure('INVALID_INPUT', 'page.pushState needs a "url".');
  let href: string;
  try {
    const target = new URL(url, tabUrl);
    if (!tabUrl || target.origin !== new URL(tabUrl).origin) {
      return failure('INVALID_INPUT', `${target.href} is not on this page’s origin — use page.navigate to leave the site.`);
    }
    href = target.href;
  } catch {
    return failure('INVALID_INPUT', `"${url}" is not a URL.`);
  }
  try {
    const [injection] = await browser.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      world: 'MAIN',
      func: pushRoute,
      args: [href],
    });
    const via = (injection?.result as Via | undefined)?.via ?? 'history';
    return success({ via, url: href });
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
}
