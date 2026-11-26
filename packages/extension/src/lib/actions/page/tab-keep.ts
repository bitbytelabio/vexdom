import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const tabKeep = defineAction({
  name: 'page.tabKeep',
  chromiumOnly: true,
  group: 'navigation',
  description:
    'Choose how a tab behind another one runs: live keeps it running as if it were in front (timers, animations, focus), frozen pauses it entirely, throttled gives it back to the browser’s normal background throttling.',
  input: z.object({
    mode: z.enum(['live', 'throttled', 'frozen']),
    tabId: z.number().int().optional().describe('The tab; the one in use when omitted'),
  }),
  execute() {
    throw new ActionError('page.tabKeep is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
