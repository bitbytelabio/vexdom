import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const initScript = defineAction({
  name: 'page.initScript',
  chromiumOnly: true,
  group: 'code',
  description:
    'Run a script in every document the tab loads from now on, before the page’s own scripts: add one (it returns an identifier), remove one by that identifier, or list them.',
  input: z.object({
    op: z.enum(['add', 'remove', 'list']),
    source: z.string().min(1).max(1_000_000).optional().describe('add: the JavaScript to run'),
    identifier: z.string().optional().describe('remove: the identifier add returned'),
  }),
  execute() {
    throw new ActionError('page.initScript is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
