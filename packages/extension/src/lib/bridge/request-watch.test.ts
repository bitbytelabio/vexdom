import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { requestsSettled, watchRequests } from './request-watch';

type Listener = (details: { requestId: string }) => void;

function fakeEvent() {
  const listeners = new Set<Listener>();
  return {
    addListener: (listener: Listener) => void listeners.add(listener),
    removeListener: (listener: Listener) => void listeners.delete(listener),
    fire: (requestId: string) => listeners.forEach((listener) => listener({ requestId })),
  };
}

const webRequest = { onBeforeRequest: fakeEvent(), onCompleted: fakeEvent(), onErrorOccurred: fakeEvent() };

beforeAll(() => {
  Object.defineProperty(fakeBrowser, 'webRequest', { value: webRequest, configurable: true });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the requests an action starts', () => {
  it('hold its snapshot until they end, and never past the cap', async () => {
    vi.useFakeTimers();
    watchRequests(4);
    webRequest.onBeforeRequest.fire('a');
    webRequest.onBeforeRequest.fire('b');
    let settled = false;
    const waiting = requestsSettled(4, 1_000).then(() => (settled = true));
    webRequest.onCompleted.fire('a');
    await vi.advanceTimersByTimeAsync(300);
    expect(settled).toBe(false);
    webRequest.onErrorOccurred.fire('b');
    await vi.advanceTimersByTimeAsync(50);
    await waiting;
    expect(settled).toBe(true);

    webRequest.onBeforeRequest.fire('long-poll');
    const capped = requestsSettled(4, 1_000).then(() => (settled = false));
    await vi.advanceTimersByTimeAsync(1_050);
    await capped;
    expect(settled).toBe(false);

    // The next action's snapshot does not wait for the long poll an earlier one started.
    watchRequests(4);
    let next = false;
    const fresh = requestsSettled(4, 1_000).then(() => (next = true));
    await vi.advanceTimersByTimeAsync(0);
    await fresh;
    expect(next).toBe(true);
  });

  it('are not waited for in a tab no action touched', async () => {
    webRequest.onBeforeRequest.fire('elsewhere');
    await expect(requestsSettled(99, 10_000)).resolves.toBeUndefined();
  });
});
