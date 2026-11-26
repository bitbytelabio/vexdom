import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { acquire, holders, onLost, release, serveDebuggerEvents } from './cdp';

type Source = { tabId: number; sessionId?: string };

const calls: string[] = [];
const detachListeners: ((source: Source, reason: string) => void)[] = [];

const fakeDebugger = {
  attach: vi.fn(async (source: Source) => void calls.push(`attach ${source.tabId}`)),
  detach: vi.fn(async (source: Source) => void calls.push(`detach ${source.tabId}`)),
  sendCommand: vi.fn(async () => ({})),
  onEvent: { addListener: () => {} },
  onDetach: { addListener: (listener: (source: Source, reason: string) => void) => detachListeners.push(listener) },
};

beforeAll(() => {
  vi.stubEnv('FIREFOX', '');
  Object.defineProperty(fakeBrowser, 'debugger', { value: fakeDebugger, configurable: true });
  serveDebuggerEvents();
});

beforeEach(() => {
  calls.length = 0;
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
  vi.stubEnv('FIREFOX', '');
});

describe('the debugger hub', () => {
  it('attaches once for two owners and detaches when the last one leaves', async () => {
    const first = await acquire(5, 'har');
    const second = await acquire(5, 'oneshot:a');
    expect(first.ok && second.ok).toBe(true);
    expect(calls).toEqual(['attach 5']);
    expect(holders(5).sort()).toEqual(['har', 'oneshot:a']);

    await release(5, 'oneshot:a');
    expect(calls).toEqual(['attach 5']);
    await release(5, 'har');
    expect(calls).toEqual(['attach 5', 'detach 5']);
    expect(holders(5)).toEqual([]);
  });

  it('tells every owner when Chrome takes the debugger away', async () => {
    await acquire(6, 'route');
    await acquire(6, 'dialog');
    const lost: [number, string][] = [];
    const stop = onLost((tabId, reason) => lost.push([tabId, reason]));
    for (const listener of detachListeners) listener({ tabId: 6 }, 'canceled_by_user');
    stop();
    expect(lost).toEqual([[6, 'canceled_by_user']]);
    expect(holders(6)).toEqual([]);
  });

  it('refuses on Firefox with the caller’s hint', async () => {
    vi.stubEnv('FIREFOX', 'true');
    await expect(acquire(7, 'har', 'no debugger here')).resolves.toEqual({
      ok: false,
      code: 'UNSUPPORTED',
      message: 'no debugger here',
    });
    expect(calls).toEqual([]);
  });

  it('reports an attach the browser refused', async () => {
    fakeDebugger.attach.mockRejectedValueOnce(new Error('Cannot access a chrome:// URL'));
    const refused = await acquire(8, 'har');
    expect(refused).toMatchObject({ ok: false, code: 'DEBUGGER_UNAVAILABLE' });
    expect(holders(8)).toEqual([]);
  });
});
