import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const dialog = defineAction({
  name: 'page.dialog',
  chromiumOnly: true,
  group: 'dialogs',
  description:
    'See or answer the alert, confirm, prompt or leave-page dialog that blocks the tab: status says whether one is open and what it asks; accept (with promptText for a prompt) or dismiss answers it; auto answers alerts and leave-page dialogs on its own from now on.',
  input: z.object({
    op: z.enum(['status', 'accept', 'dismiss', 'auto']).default('status'),
    promptText: z.string().optional().describe('accept: the text a prompt dialog gets'),
    alert: z.boolean().default(false).describe('auto: accept alerts on their own'),
    beforeunload: z.boolean().default(false).describe('auto: accept leave-page dialogs on their own'),
  }),
  execute() {
    throw new ActionError('page.dialog is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
