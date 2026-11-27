import { browser } from 'wxt/browser';
import { HAND_OFF_ACTION } from '@/lib/actions/reserved';
import { INDICATOR_CHANNEL, INDICATOR_COLOR, type IndicatorCommand } from '@/lib/indicator/events';
import { readSetting, RUN_GLOW_KEY } from '@/lib/settings/run-surface';
import { syncRail } from './rail';
import { tellTab } from './tab-message';
import { readTabSessions, type TabSession } from './tab-sessions';

type BadgeAction = {
  setBadgeText: (details: { tabId?: number; text: string }) => Promise<void> | void;
  setBadgeBackgroundColor: (details: { tabId?: number; color: string }) => Promise<void> | void;
};

const BUSY_BADGE = '●';

const IDLE: IndicatorCommand = { channel: INDICATOR_CHANNEL, op: 'idle' };

/** The last command each tab was sent, as JSON, so an unchanged tab is not told again. */
const painted = new Map<number, string>();

/** Chrome's toolbar key is `action`; Firefox MV2 calls the same surface `browserAction`. */
function badgeAction(): BadgeAction | null {
  const api = browser as unknown as { action?: BadgeAction; browserAction?: BadgeAction };
  return (import.meta.env.FIREFOX ? api.browserAction : api.action) ?? null;
}

function waitingOn(session: TabSession): { text: string; handOff: boolean } | undefined {
  const pending = session.pendingApproval;
  if (!pending) return undefined;
  const handOff = pending.action === HAND_OFF_ACTION;
  const reason = (pending.input as { reason?: unknown } | null)?.reason;
  return { handOff, text: handOff ? String(reason ?? 'Your turn.') : 'Waiting for your OK in Vexdom.' };
}

export async function syncRunIndicator(): Promise<void> {
  const glow = await readSetting(RUN_GLOW_KEY);
  const wanted = new Map<number, IndicatorCommand>();
  for (const session of Object.values(await readTabSessions())) {
    if (!session.runId) continue;
    for (const tabId of session.tabIds) wanted.set(tabId, { channel: INDICATOR_CHANNEL, op: 'busy', glow });
    const waiting = waitingOn(session);
    if (waiting) wanted.set(session.currentTabId, { channel: INDICATOR_CHANNEL, op: 'busy', glow, waiting });
  }
  for (const tabId of painted.keys()) if (!wanted.has(tabId)) wanted.set(tabId, IDLE);
  await Promise.all([...wanted].map(([tabId, command]) => paint(tabId, command)));
  await syncRail();
}

export function forgetTab(tabId: number): void {
  painted.delete(tabId);
}

/** Takes the frame and pill off a working tab while its pixels are read, then puts them back. */
export async function hushIndicator(tabId: number, hushed: boolean): Promise<void> {
  if (!painted.has(tabId)) return;
  await tellTab(tabId, { channel: INDICATOR_CHANNEL, op: hushed ? 'hush' : 'unhush' } satisfies IndicatorCommand);
}

export function serveRunIndicator(): void {
  browser.storage.local.onChanged.addListener((changes) => {
    if (RUN_GLOW_KEY in changes) void syncRunIndicator();
  });
  // A new page's content script starts bare: it is told again what the run is doing there.
  browser.tabs.onUpdated.addListener((tabId, changeInfo) => {
    const key = changeInfo.status === 'complete' ? painted.get(tabId) : undefined;
    if (key) void tellTab(tabId, JSON.parse(key) as IndicatorCommand);
  });
}

async function paint(tabId: number, command: IndicatorCommand): Promise<void> {
  const key = JSON.stringify(command);
  const busy = command.op === 'busy';
  if (busy ? painted.get(tabId) === key : !painted.has(tabId)) return;
  const wasBusy = painted.has(tabId);
  if (busy) painted.set(tabId, key);
  else painted.delete(tabId);

  const action = badgeAction();
  if (action && busy !== wasBusy) {
    try {
      await action.setBadgeBackgroundColor({ tabId, color: INDICATOR_COLOR });
      await action.setBadgeText({ tabId, text: busy ? BUSY_BADGE : '' });
    } catch {
      painted.delete(tabId);
    }
  }

  await tellTab(tabId, command);
}
