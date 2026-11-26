import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const mouse = defineAction({
  name: 'page.mouse',
  chromiumOnly: true,
  group: 'input',
  description:
    'Drive the real mouse at viewport coordinates (CSS pixels of the top document): move there, press or release a button, or turn the wheel. Events are trusted, and a held button stays down between calls, so down, move, up is a drag.',
  input: z.object({
    op: z.enum(['move', 'down', 'up', 'wheel']).describe('What the mouse does'),
    x: z.number().optional().describe('Where to (move, wheel); the current position when omitted'),
    y: z.number().optional(),
    button: z.enum(['left', 'middle', 'right']).default('left'),
    deltaX: z.number().default(0).describe('wheel: sideways scroll in pixels'),
    deltaY: z.number().default(0).describe('wheel: downward scroll in pixels'),
    human: z.boolean().default(false).describe('move: a curved, eased path at a human pace'),
    seed: z.number().int().min(0).default(0).describe('move: the bend of a human path'),
    steps: z.number().int().min(1).max(200).optional().describe('move: how many events along the way'),
    durationMs: z.number().int().min(0).max(10_000).optional().describe('move: how long the move takes'),
  }),
  execute() {
    throw new ActionError('page.mouse is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
