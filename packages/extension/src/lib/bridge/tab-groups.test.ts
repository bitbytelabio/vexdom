import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { groupLook, planGroups, requestGroupSync, type GroupInfo, type GroupTab } from './tab-groups';
import { mutateTabSessions, type TabSession } from './tab-sessions';

const session = (over: Partial<TabSession> = {}): TabSession => ({
  sessionId: 's1',
  mainTabId: 1,
  tabIds: [1, 2],
  userTabIds: [1],
  openedTabIds: [2],
  currentTabId: 1,
  windowId: 1,
  title: 'GitHub',
  host: 'github.com',
  runId: null,
  turns: 1,
  createdAt: 0,
  lastActivityAt: 0,
  ...over,
});

const tab = (id: number, over: Partial<GroupTab> = {}): GroupTab => ({
  id,
  windowId: 1,
  groupId: -1,
  pinned: false,
  ...over,
});

const plan = (over: Partial<Parameters<typeof planGroups>[0]> = {}) =>
  planGroups({
    sessions: [session()],
    mcpTabIds: [],
    tabs: [tab(1), tab(2)],
    groups: [],
    owned: {},
    enabled: true,
    ...over,
  });

describe('planGroups', () => {
  it('creates one group for an ungrouped conversation', () => {
    expect(plan()).toEqual([
      { op: 'create', owner: 's1', windowId: 1, tabIds: [1, 2], title: 'Vexdom · github.com', color: 'grey' },
    ]);
  });

  it('adds missing tabs to the group it already has and restyles it while it runs', () => {
    const groups: GroupInfo[] = [{ id: 10, windowId: 1, title: 'Vexdom · github.com', color: 'grey' }];
    expect(
      plan({
        sessions: [session({ runId: 'r' })],
        tabs: [tab(1, { groupId: 10 }), tab(2)],
        groups,
        owned: { '10': 's1' },
      }),
    ).toEqual([
      { op: 'add', groupId: 10, tabIds: [2] },
      { op: 'style', groupId: 10, title: '● Vexdom · github.com', color: 'orange' },
    ]);
  });

  it('leaves a tab the user keeps in a group of their own, but takes a tab the agent opened', () => {
    const ops = plan({
      tabs: [tab(1, { groupId: 77 }), tab(2, { groupId: 77 })],
      groups: [{ id: 77, windowId: 1, title: 'Mine' }],
    });
    expect(ops).toEqual([expect.objectContaining({ op: 'create', tabIds: [2] })]);
  });

  it('leaves pinned tabs alone', () => {
    expect(plan({ tabs: [tab(1, { pinned: true }), tab(2, { pinned: true })] })).toEqual([]);
  });

  it('ungroups a tab no conversation holds any more', () => {
    expect(
      plan({ sessions: [], tabs: [tab(5, { groupId: 10 })], groups: [{ id: 10, windowId: 1 }], owned: { '10': 's1' } }),
    ).toEqual([{ op: 'ungroup', tabIds: [5] }]);
  });

  it('only ungroups when turned off', () => {
    expect(
      plan({
        enabled: false,
        tabs: [tab(1, { groupId: 10 }), tab(2), tab(3, { groupId: 77 })],
        groups: [{ id: 10, windowId: 1 }],
        owned: { '10': 's1' },
      }),
    ).toEqual([{ op: 'ungroup', tabIds: [1] }]);
  });

  it('groups a conversation once in every window it spans', () => {
    const ops = plan({ tabs: [tab(1), tab(2, { windowId: 2 })] });
    expect(ops.map((op) => op.op === 'create' && [op.windowId, op.tabIds])).toEqual([
      [1, [1]],
      [2, [2]],
    ]);
  });

  it('puts a tab both in a conversation and opened over MCP in the conversation only', () => {
    const ops = plan({ mcpTabIds: [2, 3], tabs: [tab(1), tab(2), tab(3)] });
    expect(ops).toEqual([
      expect.objectContaining({ op: 'create', owner: 's1', tabIds: [1, 2] }),
      { op: 'create', owner: 'mcp', windowId: 1, tabIds: [3], title: 'Vexdom · MCP', color: 'purple' },
    ]);
  });
});

describe('groupLook', () => {
  it('shows what the conversation is doing, approval first', () => {
    const pending = { toolId: 't', action: 'page.submitForm', input: {} };
    expect(groupLook(session({ runId: 'r', pendingApproval: pending }))).toEqual({
      title: '⏸ Vexdom · github.com',
      color: 'yellow',
    });
    expect(groupLook(session({ outcome: 'done' }))).toEqual({ title: '✓ Vexdom · github.com', color: 'green' });
    expect(groupLook(session({ outcome: 'failed' }))).toEqual({ title: '✕ Vexdom · github.com', color: 'red' });
  });

  it('names it after its task, else its site without www, clipped to 20 characters', () => {
    expect(groupLook(session({ host: 'www.example.com' })).title).toBe('Vexdom · example.com');
    expect(groupLook(session({ host: 'a-very-long-subdomain.example.com' })).title).toBe(
      'Vexdom · a-very-long-subdoma…',
    );
    const task = { id: 't', runId: 'r', name: 'Daily digest', keepTab: false, notify: 'always' as const, startedAt: 0 };
    expect(groupLook(session({ task })).title).toBe('Vexdom · Daily digest');
  });
});

describe('requestGroupSync', () => {
  beforeEach(() => fakeBrowser.reset());
  afterEach(() => vi.restoreAllMocks());

  it('groups a conversation’s tabs in the browser and styles the group', async () => {
    const first = await fakeBrowser.tabs.create({ url: 'https://github.com/' });
    const second = await fakeBrowser.tabs.create({ url: 'https://github.com/x' });
    const group = vi.fn(async () => 42);
    const update = vi.fn(async () => undefined);
    Object.assign(fakeBrowser.tabs, { group, ungroup: vi.fn() });
    Object.assign(fakeBrowser, { tabGroups: { query: vi.fn(async () => []), update } });
    await mutateTabSessions((map) => {
      map.s1 = session({
        mainTabId: first.id!,
        currentTabId: first.id!,
        tabIds: [first.id!, second.id!],
        userTabIds: [first.id!],
      });
    });

    await requestGroupSync();

    expect(group).toHaveBeenCalledWith({
      tabIds: [first.id, second.id],
      createProperties: { windowId: first.windowId },
    });
    expect(update).toHaveBeenCalledWith(42, { title: 'Vexdom · github.com', color: 'grey' });
  });
});
