import { browser } from 'wxt/browser';
import type { ScriptPublicPath } from 'wxt/utils/inject-script';
import type { z } from 'zod';
import { focusedFrame, FRAME_GONE_HINT, TOP_FRAME } from '@/lib/bridge/frame-focus';
import { PAGE_CHANGERS } from '@/lib/bridge/seen-sites';
import type { Action } from './core';
import { ACTION_CHANNEL, failure, type ActionResult } from './protocol';

type InputArgs<I extends z.ZodType> = {} extends z.input<I> ? [input?: z.input<I>] : [input: z.input<I>];

const NO_RECEIVER = 'Receiving end does not exist';

/** Input keys that can name an element by a snapshot ref. */
const REF_KEYS = ['target', 'from', 'to', 'frame', 'container'];

function namesRef(input: unknown): boolean {
  if (typeof input !== 'object' || input === null) return false;
  return REF_KEYS.some((key) => {
    const value = (input as Record<string, unknown>)[key];
    return typeof value === 'object' && value !== null && 'ref' in value;
  });
}

export async function invokeInTab<I extends z.ZodType = z.ZodType, O = unknown>(
  tabId: number,
  action: Action<I, O> | string,
  ...[input]: InputArgs<I>
): Promise<ActionResult<O>> {
  return invokeInFrame(tabId, await focusedFrame(tabId), action, ...([input] as InputArgs<I>));
}

export async function invokeInFrame<I extends z.ZodType = z.ZodType, O = unknown>(
  tabId: number,
  frameId: number,
  action: Action<I, O> | string,
  ...[input]: InputArgs<I>
): Promise<ActionResult<O>> {
  const name = typeof action === 'string' ? action : action.name;
  const send = async () =>
    (await browser.tabs.sendMessage(
      tabId,
      { channel: ACTION_CHANNEL, action: name, input },
      { frameId },
    )) as ActionResult<O>;
  const unreachable = (detail: string): ActionResult<never> =>
    failure(
      'TAB_UNREACHABLE',
      frameId === TOP_FRAME ? `${detail} — the page may not allow content scripts` : `${detail} — ${FRAME_GONE_HINT}`,
    );

  try {
    return await send();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!message.includes(NO_RECEIVER)) return unreachable(message);
    if (!(await injectContentScript(tabId, frameId))) return unreachable(message);
    // The fresh content script never issued the refs this input names.
    if (namesRef(input)) {
      return failure(
        'STALE_REF',
        'The page reloaded since your snapshot, so its refs are gone. Take a new snapshot, then retry.',
      );
    }
    // A missing receiver usually means a new document: one the site checks before this call never
    // looked at, so an action that changes the page is not run on it unasked.
    if (PAGE_CHANGERS.has(name)) {
      return failure(
        'TAB_UNREACHABLE',
        'The page had no content script, so the action did not run; one is injected now. Take a new snapshot (the page may have changed), then retry.',
      );
    }
    // No receiver means the action never ran, so sending a read again cannot run it twice.
    try {
      return await send();
    } catch (retryError) {
      const detail = retryError instanceof Error ? retryError.message : String(retryError);
      return failure(
        'TAB_UNREACHABLE',
        `${detail} — the content script was injected but the page still did not answer`,
      );
    }
  }
}

export async function invokeInActiveTab<I extends z.ZodType = z.ZodType, O = unknown>(
  action: Action<I, O> | string,
  ...[input]: InputArgs<I>
): Promise<ActionResult<O>> {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  if (tab?.id == null) return failure('NO_ACTIVE_TAB', 'No active tab to control');
  return invokeInTab(tab.id, action, ...([input] as InputArgs<I>));
}

const INJECT_TIMEOUT_MS = 5_000;

export async function injectContentScript(tabId: number, frameId = TOP_FRAME): Promise<boolean> {
  const files = (browser.runtime.getManifest().content_scripts ?? []).flatMap(
    (script) => script.js ?? [],
  ) as ScriptPublicPath[];
  if (!files.length) return false;
  try {
    await withTimeout(
      browser.scripting.executeScript({ target: { tabId, frameIds: [frameId] }, files, injectImmediately: true }),
    );
    return true;
  } catch {
    return false;
  }
}

function withTimeout<T>(promise: Promise<T>): Promise<T> {
  return Promise.race([
    promise,
    new Promise<never>((_, reject) => setTimeout(() => reject(new Error('injection timed out')), INJECT_TIMEOUT_MS)),
  ]);
}
