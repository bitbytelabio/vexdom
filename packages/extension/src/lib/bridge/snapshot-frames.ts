/**
 * `page.snapshot` across frames, and ref-labelled screenshots. A document's snapshot names each
 * child frame by its frame id (`runtime.getFrameId`); the background takes that frame's own
 * snapshot and hangs it in the slot, up to four levels down, so the Bridge renders one tree whose
 * refs each know their frame. A ref action then goes straight to its frame.
 */
import { browser } from 'wxt/browser';
import { injectContentScript, invokeInFrame } from '@/lib/actions/client';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { snapshot, type FrameSlot, type PageSnapshot } from '@/lib/actions/page/snapshot';
import type { RefLabel } from '@/lib/frames/ref-labels';
import { CLAIM_GAP_MS, claimOf, clearLabels, describeFrame, drawLabels, TOP_FRAME } from './frame-focus';
import { screenshotTab } from './screenshot';
import { sealForPage } from './secret-vault';

/** As deep as the Bridge renders nested frames (vexdom-snapshot). */
const MAX_FRAME_DEPTH = 4;
/** A posted nonce lands a moment later: frames are asked again this often, this many times. */
const CLAIM_ROUNDS = 4;

interface SnapshotInput {
  selector?: string;
  interactive?: boolean;
  maxNodes?: number;
  includeFrames?: boolean;
}

const originOf = (url: string): string | undefined => {
  try {
    return new URL(url).origin;
  } catch {
    return undefined;
  }
};

/**
 * The tab's snapshot, from the top document down, with `tabId` so the Bridge keys refs by tab.
 * On Chrome a frame learns its id only from the background: every frame gets a content script
 * first, so it is listening when its parent posts it a nonce during the walk.
 */
export async function snapshotTab(tabId: number, input: unknown): Promise<ActionResult> {
  const asked = (input ?? {}) as SnapshotInput;
  const stitching = asked.includeFrames !== false;
  const children = stitching ? await childFrames(tabId) : [];
  const top = await invokeInFrame(tabId, TOP_FRAME, snapshot.name, asked as never);
  if (!top.ok) return top;
  const tree = top.data as PageSnapshot;
  if (stitching) {
    // A child frame is walked whole: the selector scopes the document it was asked of.
    const child: SnapshotInput = { interactive: asked.interactive, maxNodes: asked.maxNodes, includeFrames: true };
    await stitch(tabId, tree.frames, child, children, 1);
  }
  return success({ ...tree, tabId });
}

/** Every frame below the top one, each with a content script by the time this returns. */
async function childFrames(tabId: number): Promise<number[]> {
  let found: { frameId: number }[];
  try {
    found = await browser.scripting.executeScript({ target: { tabId, allFrames: true }, func: () => true });
  } catch {
    return [];
  }
  const frames = found.map(({ frameId }) => frameId).filter((frameId) => frameId !== TOP_FRAME);
  await Promise.all(
    frames.map(async (frameId) => {
      if (!(await describeFrame(tabId, frameId))) await injectContentScript(tabId, frameId);
    }),
  );
  return frames;
}

/** The frame that holds `nonce`: every frame is asked, a few times; a nonce two frames hold names neither. */
export async function claimFrame(tabId: number, nonce: string): Promise<number | undefined> {
  const slot: FrameSlot = { node: 0, probe: nonce, sameOrigin: false };
  await claim(tabId, [slot], await childFrames(tabId));
  return slot.frameId;
}

/** Names each slot's frame from the nonce it was posted; a nonce two frames claim names neither. */
async function claim(tabId: number, slots: FrameSlot[], candidates: number[]): Promise<void> {
  const waiting = new Map(slots.filter((slot) => slot.frameId === undefined && slot.probe).map((slot) => [slot.probe!, slot]));
  for (let round = 0; round < CLAIM_ROUNDS && waiting.size; round++) {
    if (round) await new Promise((resolve) => setTimeout(resolve, CLAIM_GAP_MS));
    const claims = await Promise.all(candidates.map(async (frameId) => [frameId, await claimOf(tabId, frameId)] as const));
    const owners = new Map<string, number[]>();
    for (const [frameId, nonce] of claims) if (nonce && waiting.has(nonce)) owners.set(nonce, [...(owners.get(nonce) ?? []), frameId]);
    for (const [nonce, frameIds] of owners) {
      if (frameIds.length === 1) waiting.get(nonce)!.frameId = frameIds[0];
      waiting.delete(nonce);
    }
  }
}

async function stitch(tabId: number, slots: FrameSlot[], input: SnapshotInput, candidates: number[], depth: number): Promise<void> {
  if (depth > MAX_FRAME_DEPTH) return;
  await claim(tabId, slots, candidates);
  await Promise.all(
    slots.map(async (slot) => {
      if (slot.frameId === undefined || slot.snapshot) return;
      const result = await invokeInFrame(tabId, slot.frameId, snapshot.name, input as never);
      if (!result.ok) return;
      const tree = result.data as PageSnapshot;
      await stitch(tabId, tree.frames, input, candidates, depth + 1);
      // Sealed here against its own origin: the whole-tree pass stops short of deep frames.
      slot.snapshot = (await sealForPage(tree, originOf(tree.url))).value;
    }),
  );
}

interface FrameLabel extends RefLabel {
  frameId: number;
}

function labelsOf(input: unknown): FrameLabel[] {
  const labels = (input as { labels?: unknown } | undefined)?.labels;
  if (!Array.isArray(labels)) return [];
  return labels.filter(
    (label): label is FrameLabel =>
      typeof label === 'object' &&
      label !== null &&
      typeof (label as FrameLabel).frameId === 'number' &&
      typeof (label as FrameLabel).doc === 'string' &&
      typeof (label as FrameLabel).id === 'number' &&
      typeof (label as FrameLabel).label === 'string',
  );
}

/** `page.screenshot` with `labels` (the Bridge's `--annotate`): each frame draws its own, then the capture, then they go. */
export async function screenshotWithLabels(
  tab: { id: number; windowId?: number },
  input: unknown,
  runId: string | undefined,
): Promise<ActionResult> {
  const labels = labelsOf(input);
  const { labels: _labels, ...rest } = (input ?? {}) as { labels?: unknown };
  if (!labels.length) return screenshotTab(tab, rest, runId);
  const frames = [...new Set(labels.map((label) => label.frameId))];
  await Promise.all(frames.map((frameId) => drawLabels(tab.id, frameId, labels.filter((label) => label.frameId === frameId))));
  try {
    return await screenshotTab(tab, rest, runId);
  } catch (error) {
    return failure('SCREENSHOT_FAILED', String(error));
  } finally {
    await Promise.all(frames.map((frameId) => clearLabels(tab.id, frameId)));
  }
}

/** The frame a ref target names, for actions that act on one element. */
export function refFrameOf(input: unknown): number | undefined {
  const fields = input as { target?: unknown; from?: unknown; to?: unknown } | undefined;
  for (const value of [fields?.target, fields?.from, fields?.to]) {
    const frameId = (value as { ref?: { frameId?: unknown } } | undefined)?.ref?.frameId;
    if (typeof frameId === 'number') return frameId;
  }
  return undefined;
}
