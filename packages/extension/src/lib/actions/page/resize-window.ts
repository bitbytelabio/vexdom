import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const resizeWindow = defineAction({
  name: 'page.resizeWindow',
  group: 'navigation',
  description:
    "Resize, move, maximize or minimize the browser window the agent's tab is in (not the page's viewport — use emulate for that).",
  input: z
    .object({
      width: z.number().int().min(200).max(10000).optional().describe('Window width in CSS pixels.'),
      height: z.number().int().min(200).max(10000).optional().describe('Window height in CSS pixels.'),
      left: z.number().int().optional().describe('Distance from the screen’s left edge.'),
      top: z.number().int().optional().describe('Distance from the screen’s top edge.'),
      state: z
        .enum(['normal', 'maximized', 'minimized', 'fullscreen'])
        .optional()
        .describe('Window state instead of a size; cannot be combined with width/height/left/top.'),
    })
    .superRefine((value, ctx) => {
      const geometry = [value.width, value.height, value.left, value.top].some((field) => field !== undefined);
      if (value.state !== undefined && geometry) {
        ctx.addIssue({ code: 'custom', message: 'Give a state, or a size and position — not both.' });
      } else if (value.state === undefined && !geometry) {
        ctx.addIssue({ code: 'custom', message: 'Give width, height, left, top or state.' });
      }
    }),
  execute() {
    throw new ActionError('page.resizeWindow is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
