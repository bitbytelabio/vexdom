import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const traceStart = defineAction({
  name: 'page.traceStart',
  chromiumOnly: true,
  group: 'devtools',
  description:
    'Start recording a performance trace of the tab (mode "trace": everything, for chrome://tracing or Perfetto) or a CPU profile (mode "profile": JavaScript samples and timeline). page.traceStop writes it.',
  input: z.object({
    mode: z.enum(['trace', 'profile']).default('trace'),
    categories: z.array(z.string()).optional().describe('profile: trace categories instead of the default set'),
  }),
  execute() {
    throw new ActionError('page.traceStart is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
