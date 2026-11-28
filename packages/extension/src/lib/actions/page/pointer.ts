import { z } from 'zod';
import { isOverlay } from '@/lib/overlay';
import { ActionError } from '../core';
import { cssPath } from './dom';

const APPROACH_OFFSET = 120;

export const APPROACH_STEPS = 4;
const MAX_DRIFT_PX = 6;

export interface Point {
  x: number;
  y: number;
}

export const pointSchema = z.object({
  x: z.number().describe('Pixels from the left edge of the viewport'),
  y: z.number().describe('Pixels from the top edge of the viewport'),
  source: z
    .literal('screenshot')
    .optional()
    .describe(
      'Set when you read this point off the tab’s last viewport screenshot: Vexdom scales it back to page pixels. Left out, the point is in CSS pixels.',
    ),
});

function viewport(): { w: number; h: number } {
  return { w: document.documentElement.clientWidth, h: document.documentElement.clientHeight };
}

export function elementPoint(el: HTMLElement): Point {
  const rect = el.getBoundingClientRect();
  const { w, h } = viewport();
  const left = Math.max(rect.left, 0);
  const top = Math.max(rect.top, 0);
  const right = Math.min(rect.right, w);
  const bottom = Math.min(rect.bottom, h);
  if (right <= left || bottom <= top) {
    throw new ActionError(
      'The element has no visible area inside the viewport — scroll it into view, then retry',
      'INVALID_TARGET',
    );
  }
  return { x: Math.round((left + right) / 2), y: Math.round((top + bottom) / 2) };
}

export function viewportPoint(point: Point): Point {
  const { w, h } = viewport();
  if (point.x < 0 || point.y < 0 || point.x >= w || point.y >= h) {
    throw new ActionError(
      `The point (${point.x}, ${point.y}) is outside the ${w}×${h} viewport — scroll it into view, then retry`,
      'INVALID_TARGET',
    );
  }
  return { x: Math.round(point.x), y: Math.round(point.y) };
}

export function approachPoint(to: Point): Point {
  const { w, h } = viewport();
  return {
    x: Math.min(Math.max(to.x - APPROACH_OFFSET, 0), w - 1),
    y: Math.min(Math.max(to.y - APPROACH_OFFSET, 0), h - 1),
  };
}

/**
 * What the page has at a point, looking through anything Vexdom drew over it and into open shadow
 * roots, where the document only reports their host.
 */
export function elementAt(point: Point): Element | null {
  const topmost = (root: DocumentOrShadowRoot) =>
    root.elementsFromPoint(point.x, point.y).find((element) => !isOverlay(element)) ?? null;
  let hit = topmost(document);
  while (hit?.shadowRoot) {
    const inner = topmost(hit.shadowRoot);
    if (!inner || inner === hit) break;
    hit = inner;
  }
  return hit;
}

/** `node` is `ancestor` or inside it, crossing shadow boundaries to their hosts. */
function composedContains(ancestor: Node, node: Node | null): boolean {
  for (let at = node; at; at = at instanceof ShadowRoot ? at.host : at.parentNode) {
    if (at === ancestor) return true;
  }
  return false;
}

export function assertUncovered(el: HTMLElement, at: Point): void {
  const hit = elementAt(at);
  if (hit === el || composedContains(el, hit)) return;
  // `el` is in a shadow tree under `hit` that `elementAt` could not open (a closed root).
  if (hit && composedContains(hit, el) && !hit.contains(el)) return;
  throw new ActionError(
    hit
      ? `${cssPath(hit)} covers (${at.x}, ${at.y}) — dismiss whatever is over the element, then retry`
      : `Nothing is painted at (${at.x}, ${at.y}) — scroll the element into view, then retry`,
    'INVALID_TARGET',
  );
}

const easeOut = (progress: number) => 1 - (1 - progress) ** 3;

export function pathBetween(from: Point, to: Point, steps: number): Point[] {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const span = Math.hypot(dx, dy) || 1;
  const drift = Math.min(MAX_DRIFT_PX, span / 8);
  return Array.from({ length: steps }, (_, index) => {
    const progress = easeOut((index + 1) / steps);
    const wobble = Math.sin(progress * Math.PI) * drift * (index % 2 ? -1 : 1);
    return {
      x: Math.round(from.x + dx * progress - (dy / span) * wobble),
      y: Math.round(from.y + dy * progress + (dx / span) * wobble),
    };
  });
}

const HOVER_SEQUENCE: Array<['pointer' | 'mouse', string, boolean]> = [
  ['pointer', 'pointerover', true],
  ['pointer', 'pointerenter', false],
  ['pointer', 'pointermove', true],
  ['mouse', 'mouseover', true],
  ['mouse', 'mouseenter', false],
  ['mouse', 'mousemove', true],
];

export function hoverSequence(el: Element, at: Point): void {
  for (const [kind, type, bubbles] of HOVER_SEQUENCE) {
    const Ctor = kind === 'pointer' ? PointerEvent : MouseEvent;
    el.dispatchEvent(new Ctor(type, { clientX: at.x, clientY: at.y, bubbles, composed: true }));
  }
}
