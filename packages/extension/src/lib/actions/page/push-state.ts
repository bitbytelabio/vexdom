import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const pushState = defineAction({
  name: 'page.pushState',
  group: 'navigation',
  description:
    'Change a single-page app’s route without a reload: Next.js’s router when the page has one, else history.pushState plus a popstate event its router hears. Same origin only; use page.navigate to leave the site.',
  input: z.object({
    url: z.string().min(1).describe('The new path or same-origin URL, e.g. "/settings/billing"'),
  }),
  execute() {
    throw new ActionError('page.pushState is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
