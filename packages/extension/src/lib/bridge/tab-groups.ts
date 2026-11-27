import { browser } from 'wxt/browser';
import { readSetting, TAB_GROUPS_KEY } from '@/lib/settings/run-surface';
import { adoptMcpTab, dropMcpTabs, MCP_TABS_KEY, readMcpTabs } from './mcp-tabs';
import { addTabsTo, takeTabOut } from './run-port';
import {
  clearOutcome,
  readTabSessions,
  sessionForTab,
  sessionLabel,
  TAB_SESSIONS_KEY,
  type TabSession,
} from './tab-sessions';

/** Which of the browser's tab groups Vexdom made, and for whom: a conversation's id or the MCP group. */
export const GROUP_OWNERS_KEY = 'vexdom/groupOwners';
export const MCP_OWNER = 'mcp';

export type GroupColor = 'grey' | 'blue' | 'red' | 'yellow' | 'green' | 'pink' | 'purple' | 'cyan' | 'orange';

export interface GroupTab {
  id: number;
  windowId: number;
  groupId: number;
  pinned: boolean;
}

export interface GroupInfo {
  id: number;
  windowId: number;
  title?: string;
  color?: string;
}

export type GroupOp =
  | { op: 'ungroup'; tabIds: number[] }
  | { op: 'create'; owner: string; windowId: number; tabIds: number[]; title: string; color: GroupColor }
  | { op: 'add'; groupId: number; tabIds: number[] }
  | { op: 'style'; groupId: number; title: string; color: GroupColor };

const NO_GROUP = -1;

const MCP_LOOK = { title: 'Vexdom · MCP', color: 'purple' } as const satisfies { title: string; color: GroupColor };

/** A group left behind by an earlier run of the extension, whose owners were wiped with session storage. */
const ORPHAN_TITLE = /^(?:[●⏸✓✕] )?Vexdom · /u;

/** A conversation's group: named after what it is about, coloured by what it is doing. */
export function groupLook(session: TabSession): { title: string; color: GroupColor } {
  const [glyph, color]: [string, GroupColor] = session.pendingApproval
    ? ['⏸', 'yellow']
    : session.runId
      ? ['●', 'orange']
      : session.outcome === 'done'
        ? ['✓', 'green']
        : session.outcome === 'failed'
          ? ['✕', 'red']
          : ['', 'grey'];
  return { title: `${glyph ? `${glyph} ` : ''}Vexdom · ${sessionLabel(session)}`, color };
}

/**
 * The whole grouping policy: what to do to the browser's groups so that every conversation's tabs,
 * and the tabs MCP clients opened, sit in a group of their own in each window they span.
 */
export function planGroups(input: {
  sessions: TabSession[];
  mcpTabIds: number[];
  tabs: GroupTab[];
  groups: GroupInfo[];
  owned: Record<string, string>;
  enabled: boolean;
}): GroupOp[] {
  const { sessions, mcpTabIds, tabs, groups, owned, enabled } = input;
  const ours = (groupId: number) => groupId !== NO_GROUP && String(groupId) in owned;
  if (!enabled) {
    const tabIds = tabs.filter((tab) => ours(tab.groupId)).map((tab) => tab.id);
    return tabIds.length ? [{ op: 'ungroup', tabIds }] : [];
  }

  const ownerOf = new Map<number, TabSession | typeof MCP_OWNER>();
  for (const session of sessions)
    for (const tabId of session.tabIds) if (!ownerOf.has(tabId)) ownerOf.set(tabId, session);
  for (const tabId of mcpTabIds) if (!ownerOf.has(tabId)) ownerOf.set(tabId, MCP_OWNER);

  type Wanted = { owner: string; windowId: number; tabs: GroupTab[]; look: { title: string; color: GroupColor } };
  const wanted = new Map<string, Wanted>();
  for (const tab of tabs) {
    const owner = ownerOf.get(tab.id);
    if (!owner || tab.pinned) continue;
    // The user's own group wins over ours for the tabs they put in the conversation.
    if (owner !== MCP_OWNER && owner.userTabIds.includes(tab.id) && tab.groupId !== NO_GROUP && !ours(tab.groupId))
      continue;
    const ownerId = owner === MCP_OWNER ? MCP_OWNER : owner.sessionId;
    const key = `${ownerId}\u0000${tab.windowId}`;
    const entry: Wanted = wanted.get(key) ?? {
      owner: ownerId,
      windowId: tab.windowId,
      tabs: [],
      look: owner === MCP_OWNER ? MCP_LOOK : groupLook(owner),
    };
    entry.tabs.push(tab);
    wanted.set(key, entry);
  }

  const placed = new Set<number>();
  const moves: GroupOp[] = [];
  const styles: GroupOp[] = [];
  for (const { owner, windowId, tabs: desired, look } of wanted.values()) {
    for (const tab of desired) placed.add(tab.id);
    const held = (group: GroupInfo) => desired.filter((tab) => tab.groupId === group.id).length;
    const existing = groups
      .filter((group) => owned[String(group.id)] === owner && group.windowId === windowId && held(group) > 0)
      .sort((a, b) => held(b) - held(a) || a.id - b.id)[0];
    if (!existing) {
      moves.push({ op: 'create', owner, windowId, tabIds: desired.map((tab) => tab.id), ...look });
      continue;
    }
    const joining = desired.filter((tab) => tab.groupId !== existing.id).map((tab) => tab.id);
    if (joining.length) moves.push({ op: 'add', groupId: existing.id, tabIds: joining });
    if (existing.title !== look.title || existing.color !== look.color)
      styles.push({ op: 'style', groupId: existing.id, ...look });
  }

  const leaving = tabs.filter((tab) => ours(tab.groupId) && !placed.has(tab.id)).map((tab) => tab.id);
  return [...(leaving.length ? [{ op: 'ungroup' as const, tabIds: leaving }] : []), ...moves, ...styles];
}

let queue: Promise<unknown> = Promise.resolve();

function locked<T>(task: () => Promise<T>): Promise<T> {
  const run = queue.then(task, task);
  queue = run.catch(() => undefined);
  return run;
}

/** Where each tab was last seen, kept across service-worker restarts: a drag wakes the worker. */
const SEEN_KEY = 'vexdom/groupSeen';

type Seen = Map<number, { groupId: number; windowId: number }>;

let owned: Record<string, string> | null = null;
/** Where each tab was last seen, so a group change can tell a drag out from a move to another window. */
let lastSeen: Seen | null = null;
/** Group changes not yet followed, by tab: a sync leaves those tabs alone until they are. */
const dragging = new Map<number, number>();
let pending: Promise<void> | null = null;

async function readLastSeen(): Promise<Seen> {
  if (lastSeen) return lastSeen;
  const stored = (await browser.storage.session.get(SEEN_KEY))[SEEN_KEY] as
    Record<string, { groupId: number; windowId: number }> | undefined;
  lastSeen = new Map(Object.entries(stored ?? {}).map(([tabId, seen]) => [Number(tabId), seen]));
  return lastSeen;
}

async function saveLastSeen(seen: Seen): Promise<void> {
  await browser.storage.session.set({ [SEEN_KEY]: Object.fromEntries(seen) });
}

async function readOwned(): Promise<Record<string, string>> {
  if (owned) return owned;
  const stored = await browser.storage.session.get(GROUP_OWNERS_KEY);
  owned = { ...((stored[GROUP_OWNERS_KEY] as Record<string, string> | undefined) ?? {}) };
  return owned;
}

async function writeOwned(next: Record<string, string>): Promise<void> {
  owned = next;
  await browser.storage.session.set({ [GROUP_OWNERS_KEY]: next });
}

const someTabs = (tabIds: number[]) => tabIds as [number, ...number[]];

async function apply(ops: GroupOp[], owners: Record<string, string>): Promise<void> {
  for (const op of ops) {
    try {
      if (op.op === 'ungroup') await browser.tabs.ungroup(someTabs(op.tabIds));
      else if (op.op === 'add') await browser.tabs.group({ groupId: op.groupId, tabIds: someTabs(op.tabIds) });
      else if (op.op === 'style') await browser.tabGroups.update(op.groupId, { title: op.title, color: op.color });
      else {
        const groupId = await browser.tabs.group({
          tabIds: someTabs(op.tabIds),
          createProperties: { windowId: op.windowId },
        });
        owners[String(groupId)] = op.owner;
        await browser.tabGroups.update(groupId, { title: op.title, color: op.color });
      }
    } catch {
      // A tab or group can vanish while the plan runs; the next sync sees what is left.
    }
  }
}

async function reconcile(): Promise<void> {
  const enabled = await readSetting(TAB_GROUPS_KEY);
  const sessions = Object.values(await readTabSessions());
  const mcpTabIds = await readMcpTabs();
  const tabs: GroupTab[] = (await browser.tabs.query({})).flatMap((tab) =>
    tab.id == null
      ? []
      : [{ id: tab.id, windowId: tab.windowId, groupId: tab.groupId ?? NO_GROUP, pinned: tab.pinned ?? false }],
  );
  const groups: GroupInfo[] = (await browser.tabGroups.query({})).map(({ id, windowId, title, color }) => ({
    id,
    windowId,
    title,
    color,
  }));

  const live = new Set(groups.map((group) => String(group.id)));
  const owners = Object.fromEntries(Object.entries(await readOwned()).filter(([groupId]) => live.has(groupId)));

  const open = new Set(tabs.map((tab) => tab.id));
  const held = new Set(sessions.flatMap((session) => session.tabIds));
  const stale = mcpTabIds.filter((tabId) => !open.has(tabId) || held.has(tabId));
  if (stale.length) await dropMcpTabs(stale);

  // A tab whose group change is still to be followed keeps where it was last seen, and is left
  // alone, so a sync cannot undo the drag before it is read.
  const settled = tabs.filter((tab) => !dragging.has(tab.id));
  const seen = await readLastSeen();
  for (const tabId of seen.keys()) if (!open.has(tabId)) seen.delete(tabId);
  for (const tab of settled) seen.set(tab.id, { groupId: tab.groupId, windowId: tab.windowId });
  await saveLastSeen(seen);

  const ops = planGroups({
    sessions,
    mcpTabIds: mcpTabIds.filter((id) => !stale.includes(id)),
    tabs: settled,
    groups,
    owned: owners,
    enabled,
  });
  await apply(ops, owners);
  await writeOwned(owners);
}

/** Brings the browser's groups in line with the conversations; calls that pile up while one waits share it. */
export function requestGroupSync(): Promise<void> {
  if (pending) return pending;
  pending = locked(async () => {
    pending = null;
    await reconcile();
  });
  return pending;
}

/** The user dragged a tab into or out of one of Vexdom's groups: the conversation follows. */
async function followDrag(tabId: number, groupId: number, tab: { windowId: number; pinned: boolean }): Promise<void> {
  const seen = await readLastSeen();
  const prev = seen.get(tabId);
  seen.set(tabId, { groupId, windowId: tab.windowId });
  await saveLastSeen(seen);
  if (!(await readSetting(TAB_GROUPS_KEY)) || tab.pinned) return;
  if (prev && prev.windowId !== tab.windowId) return;

  const owners = await readOwned();
  const newOwner = groupId === NO_GROUP ? undefined : owners[String(groupId)];
  const session = await sessionForTab(tabId);
  const current = session?.sessionId ?? ((await readMcpTabs()).includes(tabId) ? MCP_OWNER : undefined);

  // A tab never seen before was put in the group by the browser (a link opened from a grouped
  // tab), not dragged there by the user: it joins nothing.
  if (newOwner && newOwner !== current && prev) {
    if (newOwner === MCP_OWNER) {
      if (session && (await takeTabOut(session.sessionId, tabId)) !== 'removed') return;
      await adoptMcpTab(tabId);
    } else {
      await addTabsTo(newOwner, [tabId]);
    }
  } else if (!newOwner && prev && current && owners[String(prev.groupId)] === current) {
    if (current === MCP_OWNER) await dropMcpTabs([tabId]);
    else await takeTabOut(current, tabId);
  }
}

/** Ungroups every group an earlier run of the extension left titled as Vexdom's. */
async function sweepOrphans(groupIds?: number[]): Promise<void> {
  const ids =
    groupIds ??
    (await browser.tabGroups.query({}))
      .filter((group) => ORPHAN_TITLE.test(group.title ?? ''))
      .map((group) => group.id);
  for (const groupId of ids) {
    const tabIds = (await browser.tabs.query({ groupId })).flatMap((tab) => (tab.id == null ? [] : [tab.id]));
    if (tabIds.length) await browser.tabs.ungroup(someTabs(tabIds)).catch(() => undefined);
  }
}

export function serveTabGroups(): void {
  if (!browser.tabGroups || typeof browser.tabs.group !== 'function') return;

  void locked(async () => {
    const stored = await browser.storage.session.get(GROUP_OWNERS_KEY);
    if (stored[GROUP_OWNERS_KEY] !== undefined) return;
    await writeOwned({});
    await sweepOrphans().catch(() => undefined);
  });
  void requestGroupSync();

  const sync = () => void requestGroupSync();
  browser.storage.session.onChanged.addListener((changes) => {
    if (TAB_SESSIONS_KEY in changes || MCP_TABS_KEY in changes) sync();
  });
  browser.storage.local.onChanged.addListener((changes) => {
    if (TAB_GROUPS_KEY in changes) sync();
  });
  browser.tabs.onAttached.addListener(sync);
  browser.tabs.onRemoved.addListener((tabId) => {
    void dropMcpTabs([tabId]);
    sync();
  });
  browser.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (changeInfo.groupId !== undefined) {
      const groupId = changeInfo.groupId;
      // Marked before anything waits, so a sync already queued leaves this tab to followDrag.
      dragging.set(tabId, (dragging.get(tabId) ?? 0) + 1);
      void locked(() => followDrag(tabId, groupId, { windowId: tab.windowId, pinned: tab.pinned ?? false }))
        .finally(() => {
          const left = (dragging.get(tabId) ?? 1) - 1;
          if (left > 0) dragging.set(tabId, left);
          else dragging.delete(tabId);
        })
        .catch(() => undefined)
        .finally(sync);
    } else if (changeInfo.pinned !== undefined) sync();
  });
  browser.tabGroups.onRemoved.addListener((group) => {
    void locked(async () => {
      const owners = { ...(await readOwned()) };
      if (!(String(group.id) in owners)) return;
      delete owners[String(group.id)];
      await writeOwned(owners);
    });
  });
  browser.tabGroups.onCreated.addListener((group) => {
    void locked(async () => {
      if (ORPHAN_TITLE.test(group.title ?? '') && !(String(group.id) in (await readOwned())))
        await sweepOrphans([group.id]);
    });
  });
  browser.tabs.onActivated.addListener(({ tabId }) => {
    void sessionForTab(tabId).then((session) => (session?.outcome ? clearOutcome(session.sessionId) : undefined));
  });
}
