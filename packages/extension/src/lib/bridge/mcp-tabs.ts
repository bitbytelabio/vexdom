import { browser } from 'wxt/browser';

/** Tabs an MCP client or the CLI opened, outside any conversation; they share one tab group. */
export const MCP_TABS_KEY = 'vexdom/mcpTabs';

let queue: Promise<unknown> = Promise.resolve();

/** One read-modify-write at a time, so a tab closing while another opens loses neither. */
function change(edit: (ids: number[]) => number[] | null): Promise<void> {
  const run = queue.then(async () => {
    const next = edit(await readMcpTabs());
    if (next) await browser.storage.session.set({ [MCP_TABS_KEY]: next });
  });
  queue = run.catch(() => undefined);
  return run;
}

export async function readMcpTabs(): Promise<number[]> {
  const stored = await browser.storage.session.get(MCP_TABS_KEY);
  const ids = stored[MCP_TABS_KEY];
  return Array.isArray(ids) ? ids.filter((id): id is number => typeof id === 'number') : [];
}

export function adoptMcpTab(tabId: number): Promise<void> {
  return change((ids) => (ids.includes(tabId) ? null : [...ids, tabId]));
}

export function dropMcpTabs(tabIds: readonly number[]): Promise<void> {
  return change((ids) => {
    const kept = ids.filter((id) => !tabIds.includes(id));
    return kept.length === ids.length ? null : kept;
  });
}
