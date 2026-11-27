import { browser } from 'wxt/browser';
import { INDICATOR_COLOR } from '@/lib/indicator/events';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { isRegionCommand, type RegionPick } from './events';

const HOST_ID = 'vexdom-region';
const MIN_SIDE_PX = 8;

const STYLE = `
  :host { all: initial; }
  .backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.35); cursor: crosshair; }
  .box {
    position: fixed; display: none; pointer-events: none;
    border: 2px solid ${INDICATOR_COLOR}; box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.35);
  }
  .hint {
    position: fixed; top: 16px; left: 50%; transform: translateX(-50%); pointer-events: none;
    padding: 6px 12px; border-radius: 999px; background: rgba(20, 20, 22, 0.92); color: #f5f5f5;
    font: 500 13px/1.3 system-ui, -apple-system, 'Segoe UI', sans-serif;
  }
`;

/**
 * A box the user drags over the page, for the side panel to photograph. Only the user's own pointer
 * draws it, and it is gone from the page before the answer goes back, so the photo never shows it.
 */
export function exposeRegionPicker(): void {
  let finish: ((pick: RegionPick | null) => void) | null = null;

  browser.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
    if (!isRegionCommand(message)) return;
    if (message.op === 'cancel') {
      finish?.(null);
      sendResponse(null);
      return;
    }
    finish?.(null);
    draw((pick) => sendResponse(pick));
    return true;
  });

  function draw(answer: (pick: RegionPick | null) => void): void {
    document.getElementById(HOST_ID)?.remove();
    const host = document.createElement('div');
    host.id = HOST_ID;
    host.setAttribute(OVERLAY_ATTRIBUTE, '');
    host.style.cssText = 'all: initial; position: fixed; inset: 0; z-index: 2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    const sheet = document.createElement('style');
    sheet.textContent = STYLE;
    const backdrop = document.createElement('div');
    backdrop.className = 'backdrop';
    const box = document.createElement('div');
    box.className = 'box';
    const hint = document.createElement('div');
    hint.className = 'hint';
    hint.textContent = 'Drag to capture · Esc cancels';
    root.append(sheet, backdrop, box, hint);
    (document.body ?? document.documentElement).append(host);

    let start: { x: number; y: number } | null = null;
    let rect = { x: 0, y: 0, width: 0, height: 0 };

    const done = (pick: RegionPick | null) => {
      if (finish !== done) return;
      finish = null;
      window.removeEventListener('keydown', onKey, true);
      host.remove();
      // Two frames, so the page has painted without the overlay before it is photographed.
      requestAnimationFrame(() => requestAnimationFrame(() => answer(pick)));
    };
    finish = done;

    const onKey = (event: KeyboardEvent) => {
      if (!event.isTrusted || event.key !== 'Escape') return;
      event.preventDefault();
      event.stopPropagation();
      done(null);
    };
    window.addEventListener('keydown', onKey, true);

    backdrop.addEventListener('pointerdown', (event) => {
      if (!event.isTrusted || event.button !== 0) return;
      event.preventDefault();
      backdrop.setPointerCapture(event.pointerId);
      start = { x: event.clientX, y: event.clientY };
      hint.style.display = 'none';
    });
    backdrop.addEventListener('pointermove', (event) => {
      if (!event.isTrusted || !start) return;
      rect = {
        x: Math.min(start.x, event.clientX),
        y: Math.min(start.y, event.clientY),
        width: Math.abs(event.clientX - start.x),
        height: Math.abs(event.clientY - start.y),
      };
      Object.assign(box.style, {
        display: 'block',
        left: `${rect.x}px`,
        top: `${rect.y}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
      });
      backdrop.style.background = 'transparent';
    });
    backdrop.addEventListener('pointerup', (event) => {
      if (!event.isTrusted || !start) return;
      start = null;
      const big = rect.width >= MIN_SIDE_PX && rect.height >= MIN_SIDE_PX;
      done(big ? { rect, dpr: devicePixelRatio, viewport: { width: innerWidth, height: innerHeight } } : null);
    });
  }
}
