import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const cancelSiteTool = defineAction({
  name: 'page.cancelSiteTool',
  description:
    'Stop waiting for a site tool started with detach: true and drop whatever it returns. The site’s own code cannot be interrupted, so what it was doing may still finish.',
  input: z.object({
    invocationId: z.string().min(1).describe('The id page.callSiteTool returned'),
  }),
  execute() {
    throw new ActionError('page.cancelSiteTool is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
