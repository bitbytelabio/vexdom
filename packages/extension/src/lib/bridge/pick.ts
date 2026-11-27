import { invokeInTab } from '@/lib/actions/client';
import { pickElement } from '@/lib/actions/page/pick-element';
import { success, type ActionResult } from '@/lib/actions/protocol';
import { describeFrame, focusedFrame, frameOffset, TOP_FRAME } from './frame-focus';
import { blobToDataUrl, captureViewport } from './screenshot';
import { refusalForTab } from './site-guard';

export interface PickShot {
  dataUrl: string;
  width: number;
  height: number;
}

interface PickCapture {
  region: { x: number; y: number; w: number; h: number };
  viewport: { w: number; h: number };
  dpr: number;
}

const REGION_PADDING_CSS_PX = 16;
const MAX_SHOT_LONG_SIDE = 1200;
const SHOT_JPEG_QUALITY = 0.85;
const PAINT_SETTLE_MS = 120;

/**
 * Runs the A-Eye pick in the tab, then photographs the picked element before anything
 * can move it — the pre-screenshot that travels with the pick instead of a later re-read.
 * The capture is best-effort: a pick that cannot be photographed is still a pick.
 */
export async function pickInTab(tab: { id: number; windowId?: number }, input?: unknown): Promise<ActionResult> {
  const refused = await refusalForTab(tab.id);
  if (refused) return refused;
  const picked = await invokeInTab(tab.id, pickElement.name, input);
  if (!picked.ok) return picked;

  const moved = await refusalForTab(tab.id);
  if (moved) return moved;
  const { capture, ...data } = picked.data as { capture?: PickCapture } & Record<string, unknown>;
  const placed = capture ? await inTabViewport(tab.id, capture) : null;
  const shot = placed ? await shootRegion(tab.windowId, placed).catch(() => null) : null;
  return success(shot ? { ...data, shot } : data);
}

async function inTabViewport(tabId: number, capture: PickCapture): Promise<PickCapture | null> {
  if ((await focusedFrame(tabId)) === TOP_FRAME) return capture;
  const [offset, top] = await Promise.all([frameOffset(tabId), describeFrame(tabId, TOP_FRAME)]);
  if (!offset || !top) return null;
  return {
    ...capture,
    region: { ...capture.region, x: capture.region.x + offset.x, y: capture.region.y + offset.y },
    viewport: top.viewport,
  };
}

async function shootRegion(windowId: number | undefined, capture: PickCapture): Promise<PickShot | null> {
  await new Promise((resolve) => setTimeout(resolve, PAINT_SETTLE_MS));
  const { region, viewport, dpr } = capture;
  return cropViewport(
    windowId,
    { x: region.x, y: region.y, width: region.w, height: region.h },
    { dpr, width: viewport.w, height: viewport.h },
    { padding: REGION_PADDING_CSS_PX, maxSide: MAX_SHOT_LONG_SIDE, format: 'jpeg', quality: SHOT_JPEG_QUALITY },
  );
}

/**
 * Photographs the visible tab and cuts out `rect` (CSS pixels, plus `padding` round it, kept inside
 * the viewport), its long side scaled down to `maxSide`. Null when nothing of it is on screen.
 */
export async function cropViewport(
  windowId: number | undefined,
  rect: { x: number; y: number; width: number; height: number },
  view: { dpr: number; width: number; height: number },
  opts: { padding: number; maxSide: number; format: 'png' | 'jpeg'; quality?: number },
): Promise<PickShot | null> {
  const x = Math.max(0, rect.x - opts.padding);
  const y = Math.max(0, rect.y - opts.padding);
  const w = Math.min(view.width, rect.x + rect.width + opts.padding) - x;
  const h = Math.min(view.height, rect.y + rect.height + opts.padding) - y;
  if (w < 1 || h < 1) return null;

  const { bitmap } = await captureViewport(
    windowId,
    opts.format,
    opts.format === 'jpeg' && opts.quality !== undefined ? Math.round(opts.quality * 100) : undefined,
  );

  const scale = bitmap.width / view.width || view.dpr;
  const outScale = Math.min(1, opts.maxSide / Math.max(w * scale, h * scale, 1));
  const out = new OffscreenCanvas(
    Math.max(1, Math.round(w * scale * outScale)),
    Math.max(1, Math.round(h * scale * outScale)),
  );
  const ctx = out.getContext('2d');
  if (!ctx) {
    bitmap.close();
    return null;
  }
  ctx.drawImage(bitmap, x * scale, y * scale, w * scale, h * scale, 0, 0, out.width, out.height);
  bitmap.close();

  const type = `image/${opts.format}` as const;
  const blob = await out.convertToBlob(opts.format === 'jpeg' ? { type, quality: opts.quality } : { type });
  return { dataUrl: await blobToDataUrl(blob, type), width: out.width, height: out.height };
}
