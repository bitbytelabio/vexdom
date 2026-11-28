import { browser } from 'wxt/browser';
import { settle } from './cdp';

/** How long after an action its tab's requests are watched; past it nothing waits for them. */
const WATCH_MS = 4_000;
/** The after-action snapshot waits at most this long for requests the action started. */
export const REQUESTS_CAP_MS = 1_500;
const POLL_MS = 25;

type RequestDetails = { requestId: string };
type RequestListener = (details: RequestDetails) => void;
interface RequestEvent {
  addListener(listener: RequestListener, filter: object): void;
  removeListener(listener: RequestListener): void;
}
interface WebRequest {
  onBeforeRequest: RequestEvent;
  onCompleted: RequestEvent;
  onErrorOccurred: RequestEvent;
}

interface Watch {
  pending: Set<string>;
  until: number;
}

const watches = new Map<number, Watch>();

// Read on use: the fake browser of the tests swaps it in after import.
const webRequest = () => (browser as unknown as { webRequest?: WebRequest }).webRequest;

/**
 * Starts (or extends) watching the data requests (`fetch`, XHR) of a tab an action is about to
 * act in, so its after-action snapshot can wait for what the action asked the server for.
 */
export function watchRequests(tabId: number): void {
  const api = webRequest();
  if (!api) return;
  const known = watches.get(tabId);
  if (known) {
    // Only this action's requests count: a long poll an earlier one started must not hold it up.
    known.pending.clear();
    known.until = Date.now() + WATCH_MS;
    return;
  }
  const watch: Watch = { pending: new Set(), until: Date.now() + WATCH_MS };
  const filter = { urls: ['<all_urls>'], tabId, types: ['xmlhttprequest'] };
  const started: RequestListener = ({ requestId }) => void watch.pending.add(requestId);
  const ended: RequestListener = ({ requestId }) => void watch.pending.delete(requestId);
  api.onBeforeRequest.addListener(started, filter);
  api.onCompleted.addListener(ended, filter);
  api.onErrorOccurred.addListener(ended, filter);
  watches.set(tabId, watch);
  const expire = () => {
    const left = watch.until - Date.now();
    if (left > 0) {
      setTimeout(expire, left);
      return;
    }
    api.onBeforeRequest.removeListener(started);
    api.onCompleted.removeListener(ended);
    api.onErrorOccurred.removeListener(ended);
    watches.delete(tabId);
  };
  setTimeout(expire, WATCH_MS);
}

/** Watches on from when the action ended: typing or a wait can outlast `WATCH_MS` on its own. */
export function keepWatching(tabId: number): void {
  const known = watches.get(tabId);
  if (known) known.until = Date.now() + WATCH_MS;
}

/** Resolves once no request started since the tab's last action is in flight, or after `maxMs`. */
export async function requestsSettled(tabId: number, maxMs: number): Promise<void> {
  const deadline = Date.now() + maxMs;
  while ((watches.get(tabId)?.pending.size ?? 0) > 0 && Date.now() < deadline) await settle(POLL_MS);
}
