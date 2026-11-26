import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const checkSites = defineAction({
  name: 'page.checkSites',
  description:
    'Whether the user’s Blocked sites list allows these addresses: answers allowed, or fails with SITE_BLOCKED. The Vexdom Bridge asks before it fetches a page over HTTP for vexdom read.',
  input: z.object({
    urls: z.array(z.string()).min(1).max(20),
  }),
  execute() {
    throw new ActionError('page.checkSites is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
