import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const unrouteNetwork = defineAction({
  name: 'page.unrouteNetwork',
  chromiumOnly: true,
  group: 'network',
  description: 'Remove the route with this exact pattern, or every route of the tab when none is given.',
  input: z.object({
    pattern: z.string().optional().describe('The pattern page.routeNetwork was given'),
  }),
  execute() {
    throw new ActionError('page.unrouteNetwork is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
