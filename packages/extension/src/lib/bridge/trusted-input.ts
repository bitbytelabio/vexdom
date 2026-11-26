import { browser } from 'wxt/browser';
import { invokeInTab } from '@/lib/actions/client';
import { dragElement } from '@/lib/actions/page/drag-element';
import { APPROACH_STEPS, pathBetween, type Point } from '@/lib/actions/page/pointer';
import { trustedClick } from '@/lib/actions/page/trusted-click';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { CUE_HOST_ID } from '@/lib/cues/events';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { focusInput } from '@/lib/actions/page/focus-input';
import { pressKey } from '@/lib/actions/page/press-key';
import { SPEEDS, typeText } from '@/lib/actions/page/type-text';
import { acquire, onLost, release, send, settle, withDebugger, type DebuggerSession } from './cdp';
import { MODIFIER_BITS, charToKeyInfo, keyModifierBit, pressKeyWithChord, singleKeyEventParams } from './keys';
import { FRAME_GONE_HINT, frameOffset } from './frame-focus';

const MODIFIER_BIT: Record<string, number> = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
const BUTTON_BIT: Record<string, number> = { left: 1, right: 2, middle: 4 };

const MOVE_INTERVAL_MS = 12;

export interface ClickPlan {
  point: Point;
  from: Point;
  button: 'left' | 'right' | 'middle';
  clickCount: number;
  modifiers: string[];
  moveSteps: number;
  hoverMs: number;
  holdMs: number;
}

export interface DragPlan {
  approach: Point;
  grip: Point;
  drop: Point;
  steps: number;
  holdMs: number;
  settleMs: number;
}

const FIREFOX_HINT =
  'A trusted click needs Chrome’s debugger, which Firefox does not expose — use page.clickElement instead.';

const FIREFOX_DRAG_HINT =
  'A trusted drag needs Chrome’s debugger, which Firefox does not expose — drop "trusted" to drag with synthetic events instead.';

export function trustedClickInTab(tabId: number, input: unknown): Promise<ActionResult> {
  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    const planned = await invokeInTab(tabId, trustedClick.name, input);
    if (!planned.ok) return planned;
    const offset = await frameOffset(tabId);
    if (!offset) return failure('TAB_UNREACHABLE', `The click could not be placed — ${FRAME_GONE_HINT}`);
    const plan = planned.data as ClickPlan;
    await throughOverlays(tabId, () =>
      dispatchClick(session, { ...plan, point: shifted(plan.point, offset), from: shifted(plan.from, offset) }),
    );
    return success({ ...(planned.data as object), trusted: true });
  });
}

export function dragInTab(tabId: number, input: unknown): Promise<ActionResult> {
  return withDebugger(tabId, FIREFOX_DRAG_HINT, async (session) => {
    const planned = await invokeInTab(tabId, dragElement.name, input);
    if (!planned.ok) return planned;
    const offset = await frameOffset(tabId);
    if (!offset) return failure('TAB_UNREACHABLE', `The drag could not be placed — ${FRAME_GONE_HINT}`);
    const plan = planned.data as DragPlan;
    await throughOverlays(tabId, () =>
      dispatchDrag(session, {
        ...plan,
        approach: shifted(plan.approach, offset),
        grip: shifted(plan.grip, offset),
        drop: shifted(plan.drop, offset),
      }),
    );
    return success({ ...(planned.data as object), trusted: true });
  });
}

const shifted = (at: Point, by: Point): Point => ({ x: at.x + by.x, y: at.y + by.y });

/**
 * A real pointer lands on whatever is on top, and the hands-free mic, the rail or a toast may be
 * over the target. Inert for the length of the gesture, they let it through to the page, and an
 * agent's click can never press the mic's own stop.
 */
async function throughOverlays(tabId: number, gesture: () => Promise<void>): Promise<void> {
  await markOverlaysInert(tabId, true);
  try {
    await gesture();
  } finally {
    await markOverlaysInert(tabId, false);
  }
}

async function markOverlaysInert(tabId: number, inert: boolean): Promise<void> {
  await browser.scripting
    .executeScript({
      target: { tabId },
      func: (attribute: string, untouched: string, on: boolean) => {
        for (const host of document.querySelectorAll(`[${attribute}]:not(#${untouched})`)) host.toggleAttribute('inert', on);
      },
      args: [OVERLAY_ATTRIBUTE, CUE_HOST_ID, inert],
    })
    .catch(() => undefined);
}

export async function dispatchClick(session: DebuggerSession, plan: ClickPlan): Promise<void> {
  const { point, from, button, clickCount, modifiers, moveSteps, hoverMs, holdMs } = plan;
  const held = modifiers.reduce((mask, name) => mask | (MODIFIER_BIT[name] ?? 0), 0);
  const base = { modifiers: held, pointerType: 'mouse' };
  const dispatch = (params: Record<string, unknown>) => send(session, 'Input.dispatchMouseEvent', params);
  const moveTo = (at: Point) => dispatch({ ...base, ...at, type: 'mouseMoved', button: 'none', buttons: 0 });

  await moveTo(from);
  for (const at of pathBetween(from, point, moveSteps)) {
    await settle(MOVE_INTERVAL_MS);
    await moveTo(at);
  }
  await settle(hoverMs);

  for (let count = 1; count <= clickCount; count += 1) {
    await dispatch({ ...base, ...point, type: 'mousePressed', button, buttons: BUTTON_BIT[button], clickCount: count });
    await settle(holdMs);
    await dispatch({ ...base, ...point, type: 'mouseReleased', button, buttons: 0, clickCount: count });
  }
}

export async function dispatchDrag(session: DebuggerSession, plan: DragPlan): Promise<void> {
  const { approach, grip, drop, steps, holdMs, settleMs } = plan;
  const base = { modifiers: 0, pointerType: 'mouse' };
  const dispatch = (params: Record<string, unknown>) => send(session, 'Input.dispatchMouseEvent', params);
  const moveTo = (at: Point, buttons: number) =>
    dispatch({ ...base, ...at, type: 'mouseMoved', button: buttons ? 'left' : 'none', buttons });

  for (const at of pathBetween(approach, grip, APPROACH_STEPS)) {
    await settle(MOVE_INTERVAL_MS);
    await moveTo(at, 0);
  }
  await dispatch({ ...base, ...grip, type: 'mousePressed', button: 'left', buttons: 1, clickCount: 1 });
  await settle(holdMs);

  for (const at of pathBetween(grip, drop, steps)) {
    await settle(MOVE_INTERVAL_MS);
    await moveTo(at, 1);
  }
  await settle(settleMs);
  await dispatch({ ...base, ...drop, type: 'mouseReleased', button: 'left', buttons: 0, clickCount: 1 });
}

interface MouseState {
  x: number;
  y: number;
  buttons: number;
  modifiers: number;
}

const MOUSE_OWNER = 'mouse';
const FIREFOX_MOUSE_HINT = 'Real mouse and key events need Chrome’s debugger, which Firefox does not expose.';
const mice = new Map<number, MouseState>();
const BUTTON_MASK: Record<string, number> = { left: 1, right: 2, middle: 4 };

const primaryButton = (buttons: number) => (buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none');

/** agent-browser's `interpolated_mouse_*`: an eased path, bent by a seeded amount when `human`. */
export function planPointerMove(
  from: Point,
  to: Point,
  options: { human: boolean; seed: number; durationMs?: number; steps?: number },
): { x: number; y: number; delayMs: number }[] {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const distance = Math.hypot(dx, dy);
  const asked = options.durationMs ?? 0;
  const duration = asked === 0 && options.human ? Math.trunc(Math.min(Math.max(80 + distance * 0.35, 100), 700)) : asked;
  let steps = options.steps;
  if (steps === undefined) {
    const spatial = Math.min(Math.max(Math.ceil(distance / 12), 1), 60);
    steps = options.human && duration > 0 ? Math.max(spatial, Math.ceil(duration / 16)) : spatial;
  }
  steps = Math.min(Math.max(steps, 1), 240);
  let bend = 0;
  if (options.human && distance > 0) {
    const mask = (1n << 64n) - 1n;
    const mixed = (BigInt(options.seed) * 6364136223846793005n + 1442695040888963407n) & mask;
    const unit = Number(mixed >> 11n) / 2 ** 53;
    bend = (unit * 2 - 1) * Math.min(distance * 0.08, 36);
  }
  const [px, py] = distance > 0 ? [-dy / distance, dx / distance] : [0, 0];
  const points: { x: number; y: number; delayMs: number }[] = [];
  for (let step = 1; step <= steps; step++) {
    const delayMs = Math.floor(duration / steps) + (step <= duration % steps ? 1 : 0);
    if (step === steps) {
      points.push({ x: to.x, y: to.y, delayMs });
      continue;
    }
    const t = step / steps;
    const eased = t * t * (3 - 2 * t);
    const curve = 4 * t * (1 - t) * bend;
    points.push({ x: from.x + dx * eased + px * curve, y: from.y + dy * eased + py * curve, delayMs });
  }
  return points;
}

interface MouseInput {
  op: 'move' | 'down' | 'up' | 'wheel';
  x?: number;
  y?: number;
  button: 'left' | 'middle' | 'right';
  deltaX: number;
  deltaY: number;
  human: boolean;
  seed: number;
  steps?: number;
  durationMs?: number;
}

export async function mouseInTab(tabId: number, input: MouseInput): Promise<ActionResult> {
  const state = mice.get(tabId) ?? { x: 0, y: 0, buttons: 0, modifiers: 0 };
  const owner = state.buttons ? MOUSE_OWNER : `oneshot:${crypto.randomUUID()}`;
  const acquired = await acquire(tabId, owner, FIREFOX_MOUSE_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  const session = acquired.lease.session;
  const event = (type: string, extra: Record<string, unknown> = {}) =>
    send(session, 'Input.dispatchMouseEvent', {
      type,
      x: state.x,
      y: state.y,
      button: primaryButton(state.buttons),
      buttons: state.buttons,
      ...(state.modifiers ? { modifiers: state.modifiers } : {}),
      ...extra,
    });
  try {
    switch (input.op) {
      case 'move': {
        const to = { x: input.x ?? state.x, y: input.y ?? state.y };
        const started = Date.now();
        let elapsed = 0;
        for (const point of planPointerMove({ x: state.x, y: state.y }, to, input)) {
          state.x = point.x;
          state.y = point.y;
          await event('mouseMoved');
          elapsed += point.delayMs;
          const wait = started + elapsed - Date.now();
          if (wait > 0) await settle(wait);
        }
        mice.set(tabId, state);
        return success({ moved: true, x: to.x, y: to.y });
      }
      case 'down':
        state.buttons |= BUTTON_MASK[input.button];
        await event('mousePressed', { button: input.button, clickCount: 1 });
        mice.set(tabId, state);
        if (owner !== MOUSE_OWNER) await acquire(tabId, MOUSE_OWNER, FIREFOX_MOUSE_HINT);
        return success({ pressed: true });
      case 'up':
        state.buttons &= ~BUTTON_MASK[input.button];
        await event('mouseReleased', { button: input.button, clickCount: 1 });
        mice.set(tabId, state);
        if (!state.buttons) await release(tabId, MOUSE_OWNER);
        return success({ released: true });
      case 'wheel':
        if (input.x !== undefined) state.x = input.x;
        if (input.y !== undefined) state.y = input.y;
        await event('mouseWheel', { deltaX: input.deltaX, deltaY: input.deltaY });
        mice.set(tabId, state);
        return success({ scrolled: true, deltaX: input.deltaX, deltaY: input.deltaY });
      default:
        return failure('INVALID_INPUT', 'op is move, down, up or wheel.');
    }
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  } finally {
    if (owner !== MOUSE_OWNER) await release(tabId, owner);
  }
}

/** `page.pressKey` and `page.typeText` with `trusted: true`: real key events from the debugger. */
export async function trustedKeysInTab(tabId: number, action: string, input: Record<string, unknown>): Promise<ActionResult> {
  const target = input.target;
  if (target) {
    const clear = action === typeText.name && input.clear !== false;
    const focused = await invokeInTab(tabId, focusInput.name, { target, caret: clear ? 'all' : 'end' });
    if (!focused.ok) return focused;
  }
  return withDebugger(tabId, FIREFOX_MOUSE_HINT, async (session) => {
    const state = mice.get(tabId) ?? { x: 0, y: 0, buttons: 0, modifiers: 0 };
    const dispatch = (params: Record<string, unknown>) => send(session, 'Input.dispatchKeyEvent', params);
    if (action === pressKey.name) {
      const key = String(input.key ?? '');
      const chord = ((input.modifiers as string[] | undefined) ?? []).reduce((bits, name) => bits | (MODIFIER_BITS[name] ?? 0), 0);
      const phase = input.phase ?? 'press';
      if (phase === 'down') {
        state.modifiers |= keyModifierBit(key) ?? 0;
        await dispatch(singleKeyEventParams(key, 'keyDown', state.modifiers || undefined));
      } else if (phase === 'up') {
        state.modifiers &= ~(keyModifierBit(key) ?? 0);
        await dispatch(singleKeyEventParams(key, 'keyUp', state.modifiers || undefined));
      } else {
        await pressKeyWithChord(dispatch, key, chord, state.modifiers);
      }
      mice.set(tabId, state);
      return success({ key, modifiers: input.modifiers ?? [], phase, trusted: true });
    }
    const text = String(input.text ?? '');
    const delay = typeof input.charDelayMs === 'number' ? input.charDelayMs : SPEEDS[(input.speed as keyof typeof SPEEDS) ?? 'natural'];
    for (const ch of text) {
      if (ch === '\n' || ch === '\r' || ch === '\t') {
        const [key, code, keyCode] = charToKeyInfo(ch);
        const typed = key === 'Enter' ? '\r' : '\t';
        await dispatch({ type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, text: typed, unmodifiedText: typed });
        await dispatch({ type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode });
      } else {
        await send(session, 'Input.insertText', { text: ch });
      }
      if (delay) await settle(delay);
    }
    if (input.pressEnter === true) await pressKeyWithChord(dispatch, 'Enter', 0, state.modifiers);
    return success({ typed: [...text].length, trusted: true });
  });
}

export function serveTrustedInput(): void {
  onLost((tabId) => mice.delete(tabId));
}
