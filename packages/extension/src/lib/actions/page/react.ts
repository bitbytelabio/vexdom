import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const react = defineAction({
  name: 'page.react',
  chromiumOnly: true,
  group: 'devtools',
  description:
    'Look inside a React app: install the DevTools hook (reloads the tab; needed once), then read the component tree, inspect one component’s props, hooks and state by id, record which components re-render, or list Suspense boundaries and what they wait on.',
  input: z.object({
    op: z.enum(['install', 'tree', 'inspect', 'rendersStart', 'rendersStop', 'suspense']),
    fiberId: z.number().int().optional().describe('inspect: the component id the tree shows'),
    onlyDynamic: z.boolean().default(false).describe('suspense: only boundaries that suspend'),
    json: z.boolean().default(false).describe('The raw data instead of the text report'),
  }),
  execute() {
    throw new ActionError('page.react is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
