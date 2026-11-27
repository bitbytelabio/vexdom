import { browser, type Browser } from 'wxt/browser';
import type { ConversationTab } from '@/lib/actions/protocol';
import { sealForPage } from './secret-vault';
import { siteCheck, tabUrls } from './site-guard';
import { hostOf, type TabSession } from './tab-sessions';

const MAX_ROSTER = 20;

const MAX_ROSTER_TITLE = 120;

/** The roster each conversation's agent was last told, as JSON. */
const rosterSent = new Map<string, string>();

/** The conversation's tabs as the agent may use them: this window's, in tab-strip order, with blocked sites left out entirely. */
export async function rosterOf(
  session: TabSession,
  current: Browser.tabs.Tab | null,
): Promise<ConversationTab[] | undefined> {
  if (session.tabIds.length < 2 || current?.windowId == null) return undefined;
  const blocked = await siteCheck();
  const tabs = (await browser.tabs.query({ windowId: current.windowId }))
    .filter((tab) => tab.id != null && session.tabIds.includes(tab.id) && !blocked(...tabUrls(tab)))
    .sort((a, b) => a.index - b.index);
  const roster: ConversationTab[] = [];
  for (const tab of tabs) {
    const host = hostOf(tab.url);
    const isCurrent = tab.id === session.currentTabId;
    if (!host && !isCurrent) continue;
    const { value: sealed } = await sealForPage((tab.title ?? '').replace(/\s+/g, ' ').trim(), host);
    roster.push({
      tabId: tab.id!,
      title: sealed.slice(0, MAX_ROSTER_TITLE),
      ...(host ? { host } : {}),
      ...(isCurrent ? { current: true as const } : {}),
      ...(session.userTabIds.includes(tab.id!) ? { chosen: true as const } : {}),
    });
  }
  return roster.length > 1 ? keepingCurrent(roster) : undefined;
}

/** The first tabs in tab-strip order, up to the limit, with the one the run is on kept wherever it sits. */
function keepingCurrent(roster: ConversationTab[]): ConversationTab[] {
  const current = roster.findIndex((tab) => tab.current);
  return current < MAX_ROSTER ? roster.slice(0, MAX_ROSTER) : [...roster.slice(0, MAX_ROSTER - 1), roster[current]];
}

/** Called by startTurn with what it sent; nothing recorded means the Bridge does not take rosters. */
export function markRosterSent(sessionId: string, roster: ConversationTab[] | undefined): void {
  rosterSent.set(sessionId, JSON.stringify(roster ?? []));
}

/** True, and recorded, when `roster` differs from what was last told for this session. */
export function rosterChanged(sessionId: string, roster: ConversationTab[]): boolean {
  const before = rosterSent.get(sessionId);
  const key = JSON.stringify(roster);
  if (before === undefined || before === key) return false;
  rosterSent.set(sessionId, key);
  return true;
}

/** The conversation's tabs when they changed since its agent was last told, else undefined. */
export async function rosterUpdate(session: TabSession): Promise<ConversationTab[] | undefined> {
  if (!session.runId) return undefined;
  const current = await browser.tabs.get(session.currentTabId).catch(() => null);
  const roster = (await rosterOf(session, current)) ?? [];
  return rosterChanged(session.sessionId, roster) ? roster : undefined;
}
