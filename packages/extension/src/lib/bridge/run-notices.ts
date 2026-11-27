import { browser } from 'wxt/browser';
import { readSetting, RUN_NOTICES_KEY } from '@/lib/settings/run-surface';
import { chime } from './chime';
import { browserInFront, noticeOnDevice } from './device-notices';
import { HAND_OFF_ACTION } from '@/lib/actions/reserved';
import { approvalDetail, notify } from './task-notices';
import type { TaskVerdict } from './task-outcome';
import { readTabSessions, sessionLabel, type TabSession } from './tab-sessions';
import { goToTab, showToast } from './toast';

const NOTIFICATION_PREFIX = 'vexdom-run:';

/** The user is looking at one of these tabs: the browser is in front and one of them is the active tab there. */
export async function userIsOn(tabIds: readonly number[]): Promise<boolean> {
  if (!(await browserInFront())) return false;
  const window = await browser.windows.getLastFocused({ windowTypes: ['normal'] }).catch(() => null);
  if (window?.id == null) return false;
  const [active] = await browser.tabs.query({ active: true, windowId: window.id }).catch(() => []);
  return active?.id != null && tabIds.includes(active.id);
}

/** Who is told about a side-panel run: never a task's (they have their own notices), and only someone looking elsewhere. */
async function worthTelling(session: TabSession): Promise<boolean> {
  return !session.task && (await readSetting(RUN_NOTICES_KEY)) && !(await userIsOn(session.tabIds));
}

/**
 * A side-panel run is waiting for the user while they are elsewhere. The notice only points to the
 * conversation: what it waits on (a site for the rest of the conversation, a plan, code to read)
 * is answered there, never from a card drawn inside some other page.
 */
export async function noticeRunApproval(
  session: TabSession,
  event: { action: string; input: unknown; site?: string },
): Promise<void> {
  if (!(await worthTelling(session))) return;
  const handOff = event.action === HAND_OFF_ACTION;
  const title = handOff ? `${sessionLabel(session)}: your turn` : `${sessionLabel(session)} needs your OK`;
  const what = event.action.replace(/^(page|vexdom)\./, '');
  const body = handOff
    ? (approvalDetail(event.input) ?? 'The agent needs you to do something on the page.')
    : `It is waiting on ${what}${event.site ? ` on ${event.site}` : ''}. Answer in its side panel.`;
  await deliver(session, 'warn', title, body);
  await chime('attention');
}

/** A side-panel run ended while the user was elsewhere. A stop they asked for says nothing. */
export async function announceRunEnd(session: TabSession, verdict: TaskVerdict): Promise<void> {
  if (verdict.outcome === 'cancelled' || !(await worthTelling(session))) return;
  const ok = verdict.outcome === 'ok';
  const body = ok ? (verdict.headline ?? 'Finished.') : (verdict.reason ?? 'Ended with an error.');
  await deliver(session, ok ? 'live' : 'warn', sessionLabel(session), body);
  await chime(ok ? 'done' : 'failed');
}

/** As a task's notice goes: the computer's own notification, else a card on the page in front, else the browser's. */
async function deliver(session: TabSession, tone: 'live' | 'warn', title: string, body: string): Promise<void> {
  const { sessionId, currentTabId } = session;
  const shown =
    (await noticeOnDevice({ id: `run:${sessionId}`, kind: 'task', title, body, tone }, currentTabId)) ||
    (await showToast(
      { toastId: `run:${sessionId}:${Date.now()}`, tone, title, body, tabId: currentTabId },
      currentTabId,
    ));
  if (!shown) await notify(`${NOTIFICATION_PREFIX}${sessionId}`, title, body);
}

export function serveRunNotices(): void {
  browser.notifications?.onClicked.addListener((notificationId) => {
    if (!notificationId.startsWith(NOTIFICATION_PREFIX)) return;
    void browser.notifications.clear(notificationId).catch(() => undefined);
    void readTabSessions().then((map) => {
      const session = map[notificationId.slice(NOTIFICATION_PREFIX.length)];
      if (session) void goToTab(session.currentTabId);
    });
  });
}
