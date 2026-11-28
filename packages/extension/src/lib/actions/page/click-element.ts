import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { cssPath, describeElement, isVisible, resolveTarget, submitsOnClick, targetSchema } from './dom';
import type { Modifier } from './keyboard';
import { assertUncovered, elementPoint, type Point } from './pointer';

export type MouseButton = 'left' | 'right' | 'middle';

export interface ClickOptions {
  button?: MouseButton;
  modifiers?: readonly Modifier[];
  count?: number;
}

const BUTTON: Record<MouseButton, { button: number; buttons: number }> = {
  left: { button: 0, buttons: 1 },
  middle: { button: 1, buttons: 4 },
  right: { button: 2, buttons: 2 },
};

/**
 * The pointer and mouse sequence a user's click makes. A plain left click ends in `el.click()`, which
 * runs the element's activation the way it always has; a right click ends in `contextmenu` and a
 * middle one in `auxclick`, neither with a `click`; a double click is two clicks (`detail` 1 and 2)
 * and a `dblclick`.
 */
export function dispatchClick(el: HTMLElement, options: ClickOptions = {}): void {
  const button = options.button ?? 'left';
  const modifiers = options.modifiers ?? [];
  const count = options.count ?? 1;
  const at = clickPoint(el);
  const base = {
    bubbles: true,
    cancelable: true,
    composed: true,
    clientX: at.x,
    clientY: at.y,
    button: BUTTON[button].button,
    ctrlKey: modifiers.includes('ctrl'),
    shiftKey: modifiers.includes('shift'),
    altKey: modifiers.includes('alt'),
    metaKey: modifiers.includes('meta'),
  };
  const plain = button === 'left' && modifiers.length === 0 && count === 1;
  for (let detail = 1; detail <= count; detail += 1) {
    const down = { ...base, detail, buttons: BUTTON[button].buttons };
    const up = { ...base, detail, buttons: 0 };
    el.dispatchEvent(new PointerEvent('pointerdown', down));
    el.dispatchEvent(new MouseEvent('mousedown', down));
    el.dispatchEvent(new PointerEvent('pointerup', up));
    el.dispatchEvent(new MouseEvent('mouseup', up));
    if (button === 'right') {
      el.dispatchEvent(new MouseEvent('contextmenu', up));
    } else if (button === 'middle') {
      el.dispatchEvent(new MouseEvent('auxclick', up));
    } else if (plain && typeof el.click === 'function') {
      el.click();
    } else {
      // A ref can name an SVG or MathML element, which has no click().
      el.dispatchEvent(new MouseEvent('click', up));
    }
  }
  if (count === 2 && button === 'left')
    el.dispatchEvent(new MouseEvent('dblclick', { ...base, detail: 2, buttons: 0 }));
}

/** The centre of the element's visible part, where the uncovered check looked; its box's centre when none shows. */
function clickPoint(el: HTMLElement): Point {
  try {
    return elementPoint(el);
  } catch {
    const rect = el.getBoundingClientRect();
    return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  }
}

export const forceSchema = z.boolean().default(false).describe('Skip the visibility, enabled and covered checks.');

export function isDisabled(el: Element): boolean {
  return (el instanceof HTMLElement && el.matches(':disabled')) || el.getAttribute('aria-disabled') === 'true';
}

/**
 * What a user could not click is refused, not clicked anyway: a disabled control, a hidden
 * element, or one something else is painted over. An element outside the viewport is scrolled to
 * once first.
 */
export function assertActionable(el: HTMLElement, { visibility = true } = {}): void {
  if (isDisabled(el)) throw new ActionError(`${cssPath(el)} is disabled`, 'INVALID_TARGET');
  if (!visibility) return;
  if (!isVisible(el)) throw new ActionError(`${cssPath(el)} is hidden`, 'INVALID_TARGET');
  let point: Point;
  try {
    point = elementPoint(el);
  } catch {
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    point = elementPoint(el);
  }
  assertUncovered(el, point);
}

/** How often an element that is not actionable yet is looked at again. */
const ACTIONABLE_POLL_MS = 50;

/**
 * `assertActionable`, retried while the element is disabled, hidden or covered and `timeoutMs`
 * lasts: a button that enables a moment later, or a toast that fades, should not fail the action.
 * Polls on a timer, as background tabs run no animation frames. Free when it is actionable already.
 */
export async function waitActionable(
  el: HTMLElement,
  timeoutMs: number,
  options: { visibility?: boolean } = {},
): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  for (;;) {
    if (!el.isConnected) {
      throw new ActionError('The element left the page while Vexdom waited for it. Take a new snapshot.', 'STALE_REF');
    }
    try {
      assertActionable(el, options);
      return;
    } catch (error) {
      const waiting = error instanceof ActionError && error.code === 'INVALID_TARGET' && performance.now() < deadline;
      if (!waiting) throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, ACTIONABLE_POLL_MS));
  }
}

export const actionableTimeoutSchema = z
  .number()
  .int()
  .min(0)
  .max(30_000)
  .default(2_000)
  .describe('How long to wait for the element to become enabled, visible and uncovered');

export const clickElement = defineAction({
  name: 'page.clickElement',
  description:
    'Click an element like a user would, firing the full pointer and mouse event sequence: a left, right (context menu) or middle click, a double click, with modifier keys held.',
  input: z.object({
    target: targetSchema.describe('Element to click'),
    scrollIntoView: z.boolean().default(true).describe('Bring the element into view before clicking'),
    button: z.enum(['left', 'right', 'middle']).default('left').describe('Mouse button'),
    modifiers: z
      .array(z.enum(['ctrl', 'shift', 'alt', 'meta']))
      .default([])
      .describe('Modifier keys held during the click'),
    count: z.number().int().min(1).max(2).default(1).describe('2 for a double click'),
    force: forceSchema,
    timeoutMs: actionableTimeoutSchema,
  }),
  async execute({ target, scrollIntoView, button, modifiers, count, force, timeoutMs }) {
    const el = resolveTarget(target);
    if (scrollIntoView) el.scrollIntoView({ block: 'center', behavior: 'instant' });
    if (!force) await waitActionable(el, timeoutMs);
    try {
      el.focus({ preventScroll: true });
    } catch {}
    const submits = submitsOnClick(el);
    dispatchClick(el, { button, modifiers, count });
    return { ...describeElement(el), submits };
  },
});
