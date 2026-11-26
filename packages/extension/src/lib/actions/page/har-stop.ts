import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const harStop = defineAction({
  name: 'page.harStop',
  chromiumOnly: true,
  group: 'network',
  description:
    'Stop the HAR recording page.harStart began and write it as a HAR 1.2 file: to "path" when given (absolute), else into ~/vexdom/captures. Reports the path, its size and how many requests it holds.',
  input: z.object({
    path: z.string().optional().describe('Absolute path of the .har file to write'),
    inline: z.boolean().default(false).describe('Return the HAR itself instead of writing a file'),
  }),
  execute() {
    throw new ActionError('page.harStop is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
