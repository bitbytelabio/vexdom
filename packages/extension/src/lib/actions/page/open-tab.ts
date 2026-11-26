import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const openTab = defineAction({
  name: 'page.openTab',
  description:
    'Open a URL in a new browser tab. The new tab becomes the one every later page action targets, unless "active" is false.',
  input: z.object({
    url: z.string().describe('Absolute or relative URL to open in the new tab (http/https only).'),
    active: z
      .boolean()
      .default(true)
      .describe(
        'Bring the new tab to the front, making it the tab later page actions target. Set false to open it in the background and leave the current tab in front.',
      ),
    label: z
      .string()
      .optional()
      .describe('A name to come back to this tab by (switchTab or closeTab "label"): a letter, then letters, digits, - or _.'),
  }),
  execute() {
    throw new ActionError('page.openTab is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
