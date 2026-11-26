import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const pdf = defineAction({
  name: 'page.pdf',
  chromiumOnly: true,
  group: 'capture',
  description:
    'Save the page as a PDF: to "path" when given (absolute), else into ~/vexdom/captures. Reports where it went and its size.',
  input: z.object({
    path: z.string().optional().describe('Absolute path of the .pdf to write'),
    landscape: z.boolean().default(false),
    printBackground: z.boolean().default(true).describe('Print background colours and images'),
    preferCSSPageSize: z.boolean().default(false).describe('Use the page size the CSS declares'),
  }),
  execute() {
    throw new ActionError('page.pdf is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
