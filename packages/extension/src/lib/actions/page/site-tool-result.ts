import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const siteToolResult = defineAction({
  name: 'page.siteToolResult',
  description:
    'The outcome of a site tool started with page.callSiteTool and detach: true — running, done (with what it returned), failed or cancelled — waiting up to waitMs for it to settle.',
  input: z.object({
    invocationId: z.string().min(1).describe('The id page.callSiteTool returned'),
    waitMs: z.number().int().min(0).max(120_000).default(0).describe('How long to wait for a running call to settle'),
  }),
  execute() {
    throw new ActionError('page.siteToolResult is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
