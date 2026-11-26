import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const traceStop = defineAction({
  name: 'page.traceStop',
  chromiumOnly: true,
  group: 'devtools',
  description:
    'Stop the trace or profile page.traceStart began and write it as JSON: to "path" when given (absolute), else into ~/vexdom/captures.',
  input: z.object({
    path: z.string().optional().describe('Absolute path of the file to write'),
  }),
  execute() {
    throw new ActionError('page.traceStop is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
