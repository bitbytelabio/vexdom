import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const openWindow = defineAction({
  name: 'page.openWindow',
  group: 'navigation',
  description: 'Open a new browser window, on a URL or the browser’s new-tab page, and make its tab the one you work in.',
  input: z.object({
    url: z.string().optional().describe('Absolute http(s) URL to open; leave out for a new-tab page'),
  }),
  execute() {
    throw new ActionError('page.openWindow is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
