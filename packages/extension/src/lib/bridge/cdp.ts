import { browser } from 'wxt/browser';
import { failure, type ActionResult } from '@/lib/actions/protocol';

export interface DebuggerSession {
  tabId: number;
  sessionId?: string;
}

interface AttachedTarget {
  sessionId: string;
  targetInfo: { url: string; type: string };
}

interface SessionDebugger {
  attach: (target: DebuggerSession, version: string) => Promise<void>;
  detach: (target: DebuggerSession) => Promise<void>;
  sendCommand: (target: DebuggerSession, method: string, params?: object) => Promise<object | undefined>;
  onEvent: {
    addListener: (listener: (source: DebuggerSession, method: string, params?: object) => void) => void;
  };
  onDetach: {
    addListener: (listener: (source: DebuggerSession, reason: string) => void) => void;
  };
}

type EventListener = (source: DebuggerSession, method: string, params?: object) => void;

// Read on use: the fake browser of the tests swaps it in after import.
const cdp = () => browser.debugger as unknown as SessionDebugger;

const PROTOCOL_VERSION = '1.3';
const INFOBAR_SETTLE_MS = 250;
const ATTACH_SETTLE_MS = 150;
const MAX_FRAME_DEPTH = 4;

export const settle = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const listeners = new Set<EventListener>();
const lostListeners = new Set<(tabId: number, reason: string) => void>();

/**
 * One debugger attachment per tab, shared by every feature that needs it (ADR-030). Each holds a
 * lease named by its owner — `oneshot:<uuid>` for one action, `diagnostics:<id>`, `route`, `har`,
 * `dialog`, `emulation`, `init`, `record`, `trace`, `keep`, `react`, `mouse` for the long-lived
 * ones. The first lease attaches, the last release detaches; Chrome taking the debugger away
 * (DevTools, the infobar's Cancel) or the tab closing ends every lease at once, and `onLost`
 * tells each owner so it can drop its state for that tab.
 */
const leases = new Map<number, Set<string>>();
const locks = new Map<number, Promise<unknown>>();

export interface Lease {
  tabId: number;
  owner: string;
  session: DebuggerSession;
}

export type Acquired =
  | { ok: true; lease: Lease }
  | { ok: false; code: 'UNSUPPORTED' | 'DEBUGGER_UNAVAILABLE'; message: string };

const FIREFOX_DEFAULT_HINT = 'This needs Chrome’s debugger, which Firefox does not offer to extensions.';

/** Runs attach and detach for one tab one at a time. */
function locked<T>(tabId: number, run: () => Promise<T>): Promise<T> {
  const next = (locks.get(tabId) ?? Promise.resolve()).then(run, run);
  locks.set(
    tabId,
    next.catch(() => undefined),
  );
  return next;
}

export function acquire(tabId: number, owner: string, firefoxHint = FIREFOX_DEFAULT_HINT): Promise<Acquired> {
  if (import.meta.env.FIREFOX) return Promise.resolve({ ok: false, code: 'UNSUPPORTED', message: firefoxHint });
  return locked(tabId, async (): Promise<Acquired> => {
    let owners = leases.get(tabId);
    if (!owners) {
      try {
        await cdp().attach({ tabId }, PROTOCOL_VERSION);
      } catch (error) {
        return { ok: false, code: 'DEBUGGER_UNAVAILABLE', message: attachHint(error) };
      }
      owners = new Set();
      leases.set(tabId, owners);
      await settle(INFOBAR_SETTLE_MS);
    }
    owners.add(owner);
    return { ok: true, lease: { tabId, owner, session: { tabId } } };
  });
}

/** Ends one owner's lease; the last one detaches. Never disables a domain another owner relies on. */
export function release(tabId: number, owner: string): Promise<void> {
  if (import.meta.env.FIREFOX) return Promise.resolve();
  return locked(tabId, async () => {
    const owners = leases.get(tabId);
    if (owners) {
      owners.delete(owner);
      if (owners.size) return;
      leases.delete(tabId);
    }
    const session: DebuggerSession = { tabId };
    await send(session, 'Target.setAutoAttach', AUTO_ATTACH_OFF).catch(() => {});
    await cdp()
      .detach(session)
      .catch(() => {});
  });
}

export function holders(tabId: number): string[] {
  return [...(leases.get(tabId) ?? [])];
}

/** Every lease on a tab ended without a release: `reason` is Chrome's, or `tab-closed`. */
export function onLost(listener: (tabId: number, reason: string) => void): () => void {
  lostListeners.add(listener);
  return () => lostListeners.delete(listener);
}

function lost(tabId: number, reason: string): void {
  if (!leases.delete(tabId)) return;
  for (const listener of lostListeners) listener(tabId, reason);
}

export function serveDebuggerEvents(): void {
  if (import.meta.env.FIREFOX) return;
  cdp().onEvent.addListener((source, method, params) => {
    for (const listener of listeners) listener(source, method, params);
  });
  cdp().onDetach.addListener((source, reason) => {
    if (source.tabId != null && !source.sessionId) lost(source.tabId, reason);
  });
  browser.tabs.onRemoved.addListener((tabId) => lost(tabId, 'tab-closed'));
}

export function onDebuggerEvent(listener: EventListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function send<T = unknown>(
  session: DebuggerSession,
  method: string,
  params?: Record<string, unknown>,
): Promise<T> {
  return cdp().sendCommand(session, method, params) as Promise<T>;
}

/** One action's lease: attach (or share), run, release. */
export async function withDebugger(
  tabId: number,
  firefoxHint: string,
  run: (session: DebuggerSession) => Promise<ActionResult>,
): Promise<ActionResult> {
  const owner = `oneshot:${crypto.randomUUID()}`;
  const acquired = await acquire(tabId, owner, firefoxHint);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  try {
    return await run(acquired.lease.session);
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  } finally {
    await release(tabId, owner);
  }
}

/** `DEBUGGER_DETACHED` for a feature whose lease on a tab was lost. */
export function detachedFailure(reason: string): ActionResult {
  return failure('DEBUGGER_DETACHED', `the debugger left this tab (${reason}); call again to re-attach`);
}

export interface FrameSession {
  sessionId: string;
  url: string;
  parent?: string;
}

const AUTO_ATTACH = {
  autoAttach: true,
  flatten: true,
  waitForDebuggerOnStart: false,
  filter: [{ type: 'iframe' }],
};

// Auto-attached child sessions outlive the root detach, and Chrome then stalls the next
// attach on that tab instead of refusing it — a 30 s TIMEOUT rather than an error.
const AUTO_ATTACH_OFF = { autoAttach: false, flatten: true, waitForDebuggerOnStart: false };

export async function frameSessions(session: DebuggerSession): Promise<FrameSession[]> {
  const found: FrameSession[] = [];
  const stop = onDebuggerEvent((source, method, params) => {
    if (method !== 'Target.attachedToTarget') return;
    const attached = params as AttachedTarget;
    if (attached?.targetInfo?.type !== 'iframe') return;
    found.push({ sessionId: attached.sessionId, url: attached.targetInfo.url, parent: source.sessionId });
  });

  try {
    let frontier = [session];
    for (let depth = 0; depth < MAX_FRAME_DEPTH && frontier.length; depth += 1) {
      const known = found.length;
      for (const target of frontier) await send(target, 'Target.setAutoAttach', AUTO_ATTACH).catch(() => {});
      await settle(ATTACH_SETTLE_MS);
      frontier = found.slice(known).map(({ sessionId }) => ({ tabId: session.tabId, sessionId }));
    }
  } finally {
    stop();
  }
  return found;
}

function attachHint(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error);
  return /already attached|another debugger/i.test(detail)
    ? `${detail} — close DevTools for this tab, then retry.`
    : `${detail} — Chrome’s debugger could not attach to this tab.`;
}
