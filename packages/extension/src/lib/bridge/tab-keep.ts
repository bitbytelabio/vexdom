/**
 * `page.tabKeep` (agent-browser `tab keep`): `live` emulates focus so a background tab keeps
 * running as if in front, `frozen` pauses a background tab's lifecycle, `throttled` undoes both.
 * A frozen tab is thawed before any other action reaches it.
 */
import { browser } from 'wxt/browser';
import { z } from 'zod';
import { tabKeep } from '@/lib/actions/page/tab-keep';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, onLost, release, send } from './cdp';

const OWNER = 'keep';
const FIREFOX_HINT = 'Keeping a tab live or frozen needs Chrome’s debugger, which Firefox does not offer to extensions.';

const kept = new Map<number, 'live' | 'frozen'>();

export async function tabKeepInTab(currentTabId: number, input: unknown): Promise<ActionResult> {
  const parsed = tabKeep.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { mode } = parsed.data;
  const tabId = parsed.data.tabId ?? currentTabId;

  if (mode === 'throttled') {
    const was = kept.get(tabId);
    if (was) {
      await revert(tabId, was);
      kept.delete(tabId);
      await release(tabId, OWNER);
    }
    return success({ tabId, keep: 'throttled' });
  }

  if (mode === 'frozen') {
    const tab = await browser.tabs.get(tabId).catch(() => undefined);
    if (!tab) return failure('TARGET_NOT_FOUND', `No tab with id ${tabId}.`);
    if (tab.active) {
      return failure('TAB_VISIBLE', `Tab ${tabId} is visible; only a tab hidden behind another tab can be frozen`);
    }
  }
  const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  const was = kept.get(tabId);
  if (was && was !== mode) await revert(tabId, was);
  try {
    if (mode === 'live') await send({ tabId }, 'Emulation.setFocusEmulationEnabled', { enabled: true });
    else await send({ tabId }, 'Page.setWebLifecycleState', { state: 'frozen' });
  } catch (error) {
    if (!was) await release(tabId, OWNER);
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
  kept.set(tabId, mode);
  return success({ tabId, keep: mode });
}

async function revert(tabId: number, mode: 'live' | 'frozen'): Promise<void> {
  if (mode === 'live') await send({ tabId }, 'Emulation.setFocusEmulationEnabled', { enabled: false }).catch(() => undefined);
  else await send({ tabId }, 'Page.setWebLifecycleState', { state: 'active' }).catch(() => undefined);
}

/** Wakes a frozen tab before an action reaches it. */
export async function thawIfFrozen(tabId: number): Promise<void> {
  if (kept.get(tabId) !== 'frozen') return;
  await revert(tabId, 'frozen');
  kept.delete(tabId);
  await release(tabId, OWNER);
}

export function serveTabKeep(): void {
  onLost((tabId) => kept.delete(tabId));
}
