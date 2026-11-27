import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { UNDESCRIBED } from './dom';

export const recordStart = defineAction({
  name: 'page.recordStart',
  chromiumOnly: true,
  group: 'capture',
  description:
    'Record a video of the tab into "path" (absolute, .webm, .mp4 or .gif) until page.recordStop. The Bridge encodes it with ffmpeg, which must be installed. contactSheet also writes a PNG grid of the moments the page changed.',
  input: z.object({
    path: z
      .string()
      .min(1)
      .describe(
        'Absolute path ending in .webm, .mp4 or .gif. A .gif is made after you stop: at most 12 fps, 960 px wide, with a progress bar along the bottom.',
      ),
    fps: z.number().int().min(1).max(60).default(30),
    cursor: z
      .boolean()
      .default(false)
      .describe(
        'Draw the mouse pointer and its clicks into the video. While recording, every action the agent takes is ringed and captioned in the video.',
      ),
    contactSheet: z.boolean().default(false),
    contactSheetThreshold: z
      .number()
      .min(0)
      .max(1)
      .optional()
      .describe('Share of pixels that must change for a frame to join the sheet'),
    recordingId: z
      .string()
      .optional()
      .meta({ [UNDESCRIBED]: true }),
  }),
  execute() {
    throw new ActionError('page.recordStart is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
