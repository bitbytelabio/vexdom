import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const evaluate = defineAction({
  name: 'page.evaluate',
  group: 'code',
  description:
    'Run JavaScript in the page (the frame in focus) and get its value back as JSON: an expression, or statements ending in `return`; `await` works. It runs in the page’s own world (page globals, the page’s rules), which needs “Allow User Scripts” switched on for Vexdom.',
  input: z.object({
    code: z.string().min(1).max(100_000).describe('The JavaScript to run'),
    timeoutMs: z.number().int().min(100).max(120_000).default(10_000).describe('How long the code may run'),
  }),
  execute() {
    throw new ActionError('page.evaluate is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
