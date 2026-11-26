import { browser } from 'wxt/browser';
import { FRAME_CHANNEL, type FrameDescription, type FrameOrigin, type FrameProbeBody } from '@/lib/frames/events';
import type { RefLabel } from '@/lib/frames/ref-labels';

export const FRAME_FOCUS_KEY = 'vexdom/frameFocus';

export const TOP_FRAME = 0;

/** A posted nonce lands a moment later: frames are asked again after this long. */
export const CLAIM_GAP_MS = 50;

export interface FrameStep {
  frameId: number;
  url: string;
  selector: string;
}

type FocusMap = Record<string, FrameStep[]>;

let cache: FocusMap | null = null;

async function readFocus(): Promise<FocusMap> {
  if (cache) return cache;
  const stored = await browser.storage.session.get(FRAME_FOCUS_KEY);
  cache = (stored[FRAME_FOCUS_KEY] as FocusMap | undefined) ?? {};
  return cache;
}

async function writeFocus(next: FocusMap): Promise<void> {
  cache = next;
  await browser.storage.session.set({ [FRAME_FOCUS_KEY]: next });
}

export async function framePath(tabId: number): Promise<FrameStep[]> {
  return (await readFocus())[String(tabId)] ?? [];
}

export async function focusedFrame(tabId: number): Promise<number> {
  return (await framePath(tabId)).at(-1)?.frameId ?? TOP_FRAME;
}

export async function focusedUrl(tabId: number, fallback?: string): Promise<string | undefined> {
  return (await framePath(tabId)).at(-1)?.url ?? fallback;
}

export async function setFramePath(tabId: number, path: FrameStep[]): Promise<void> {
  const { [String(tabId)]: _previous, ...rest } = await readFocus();
  await writeFocus(path.length ? { ...rest, [String(tabId)]: path } : rest);
}

export function forgetFrameFocus(tabId: number): Promise<void> {
  return setFramePath(tabId, []);
}

export function serveFrameFocus(): void {
  browser.tabs.onRemoved.addListener((tabId) => void forgetFrameFocus(tabId));
  browser.tabs.onUpdated.addListener((tabId, change) => {
    if (change.status === 'loading') void forgetFrameFocus(tabId);
  });
}

async function probe<T>(tabId: number, frameId: number, message: FrameProbeBody): Promise<T | null> {
  try {
    const reply = await browser.tabs.sendMessage(tabId, { channel: FRAME_CHANNEL, ...message }, { frameId });
    return (reply as T | undefined) ?? null;
  } catch {
    return null;
  }
}

export const describeFrame = (tabId: number, frameId: number) =>
  probe<FrameDescription>(tabId, frameId, { op: 'describe' });

export const markFrame = (tabId: number, frameId: number, token: string) =>
  probe<{ ok: true }>(tabId, frameId, { op: 'mark', token });

export const unmarkFrame = (tabId: number, frameId: number) => probe<{ ok: true }>(tabId, frameId, { op: 'unmark' });

export const drawLabels = (tabId: number, frameId: number, labels: RefLabel[]) =>
  probe<{ ok: true; drawn: number }>(tabId, frameId, { op: 'labels', labels });

export const clearLabels = (tabId: number, frameId: number) => probe<{ ok: true }>(tabId, frameId, { op: 'unlabel' });

/** The nonce a frame's parent last posted into it, or null; null too for a frame with no content script. */
export const claimOf = async (tabId: number, frameId: number) =>
  (await probe<{ nonce: string | null }>(tabId, frameId, { op: 'claim' }))?.nonce ?? null;

/** Where a child frame sits in its parent: the parent announces into its frames when the child holds no nonce yet. */
async function locateFrame(tabId: number, parent: number, frameId: number): Promise<FrameOrigin | null> {
  let nonce = await claimOf(tabId, frameId);
  if (!nonce) {
    await probe(tabId, parent, { op: 'announceAll' });
    await new Promise((resolve) => setTimeout(resolve, CLAIM_GAP_MS));
    nonce = await claimOf(tabId, frameId);
  }
  if (!nonce) return null;
  return probe<FrameOrigin>(tabId, parent, { op: 'locate', probe: nonce });
}

/** Where the focused frame's viewport origin sits in the tab's viewport, in CSS pixels. */
export async function frameOffset(tabId: number): Promise<FrameOrigin | null> {
  const path = await framePath(tabId);
  let origin: FrameOrigin = { x: 0, y: 0 };
  let parent = TOP_FRAME;
  for (const { frameId } of path) {
    const child = await locateFrame(tabId, parent, frameId);
    if (!child) return null;
    origin = { x: origin.x + child.x, y: origin.y + child.y };
    parent = frameId;
  }
  return origin;
}

export const FRAME_GONE_HINT =
  'the frame in focus is gone or blocks scripts — call page.switchFrame with no arguments to return to the top document';
