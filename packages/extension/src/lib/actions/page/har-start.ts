import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const harStart = defineAction({
  name: 'page.harStart',
  chromiumOnly: true,
  group: 'network',
  description:
    'Start recording the tab’s requests and responses as a HAR. "text" keeps the bodies of text responses (JSON, HTML, scripts), "all" every body, "none" only sizes and types. page.harStop writes the file.',
  input: z.object({
    content: z.enum(['all', 'text', 'none']).default('text').describe('Which response bodies to keep'),
  }),
  execute() {
    throw new ActionError('page.harStart is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
