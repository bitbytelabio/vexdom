import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const recordStop = defineAction({
  name: 'page.recordStop',
  chromiumOnly: true,
  group: 'capture',
  description: 'Stop the video page.recordStart began; reports the file, its frames and length, and the contact sheet if asked for.',
  input: z.object({}),
  execute() {
    throw new ActionError('page.recordStop is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
