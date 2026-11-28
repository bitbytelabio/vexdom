import { z } from 'zod';
import { defineAction } from '../core';
import { resolveTarget, targetSchema } from './dom';

export const scrollTo = defineAction({
  name: 'page.scrollTo',
  description:
    'Scroll the page, or one scrollable element, to an element, an absolute position, or by pixels or one viewport in a direction.',
  input: z.object({
    target: targetSchema.optional().describe('Element to bring into view'),
    position: z
      .object({
        x: z.number().default(0).describe('Document x coordinate'),
        y: z.number().describe('Document y coordinate'),
      })
      .optional()
      .describe('Absolute document coordinates to scroll to'),
    direction: z
      .enum(['up', 'down', 'left', 'right', 'top', 'bottom'])
      .optional()
      .describe('Scroll one viewport (or "amount" pixels) in a direction, or jump to an edge'),
    amount: z.number().positive().optional().describe('Pixels to scroll for up, down, left or right'),
    container: targetSchema
      .optional()
      .describe('A scrollable element to scroll instead of the window, such as a list or a side panel'),
    behavior: z.enum(['smooth', 'instant']).default('instant').describe('Animation of the scroll'),
  }),
  async execute({ target, position, direction, amount, container, behavior }) {
    const box = container ? resolveTarget(container) : null;
    const scroller: { scrollTo(options: ScrollToOptions): void } = box ?? window;
    const at = () =>
      box ? { left: box.scrollLeft, top: box.scrollTop } : { left: window.scrollX, top: window.scrollY };
    const view = box
      ? { width: box.clientWidth, height: box.clientHeight, full: box.scrollHeight }
      : { width: window.innerWidth, height: window.innerHeight, full: document.documentElement.scrollHeight };
    const settled = scrollSettled(box ?? window);

    if (target) {
      resolveTarget(target).scrollIntoView({ behavior, block: 'center' });
    } else if (position) {
      scroller.scrollTo({ left: position.x, top: position.y, behavior });
    } else {
      const { left, top } = at();
      const step = (size: number) => amount ?? size;
      const destination: ScrollToOptions = {
        up: { top: top - step(view.height) },
        down: { top: top + step(view.height) },
        left: { left: left - step(view.width) },
        right: { left: left + step(view.width) },
        top: { top: 0 },
        bottom: { top: view.full },
      }[direction ?? 'down'];
      scroller.scrollTo({ ...destination, behavior });
    }
    if (behavior === 'smooth') await settled.done;
    else settled.cancel();
    const { left, top } = at();
    return { scrollX: Math.round(left), scrollY: Math.round(top) };
  },
});

function scrollSettled(on: EventTarget): { done: Promise<void>; cancel(): void } {
  let finish = () => {};
  const done = new Promise<void>((resolve) => {
    const timer = setTimeout(() => finish(), 1000);
    finish = () => {
      on.removeEventListener('scrollend', finish);
      clearTimeout(timer);
      resolve();
    };
    on.addEventListener('scrollend', finish);
  });
  return { done, cancel: () => finish() };
}
