import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { targetSchema } from './dom';

export const CAPTURE_TIMEOUT_MS = 60_000;

export const captureDownload = defineAction({
  name: 'page.captureDownload',
  group: 'downloads',
  description:
    'Make the page download a file and keep it. Click something that produces a download — an “Export CSV” button, a “Download invoice” link — or give a direct url, which is fetched in the browser’s own logged-in session rather than anonymously, or give neither to wait for the next download this tab starts. The file lands in the user’s ~/vexdom/download/ folder and the result reports the path and notes about what arrived; you get the notes, never the bytes. Hand the returned downloadId to page.attachFile to upload it somewhere else without the file ever passing through you.',
  input: z.object({
    target: targetSchema
      .optional()
      .describe('The link or button whose click starts the download. Give this or "url", not both; neither waits for the next download.'),
    url: z
      .string()
      .optional()
      .describe(
        'Direct http(s) url of the file, fetched with the browser’s cookies. Give this or "target", not both. Prefer "target" when a button exists: many exports have no fetchable url at all.',
      ),
    timeoutMs: z
      .number()
      .int()
      .min(1_000)
      .max(600_000)
      .default(CAPTURE_TIMEOUT_MS)
      .describe('How long to wait for the download to finish before giving up.'),
  }),
  execute() {
    throw new ActionError(
      'page.captureDownload is resolved by the Vexdom extension, not in the page',
      'UNSUPPORTED',
    );
  },
});

/** What starts the download: a click, a fetch, or (neither given) whatever the tab does next. */
export function resolveTrigger(input: { target?: unknown; url?: unknown }): 'click' | 'url' | 'next' {
  const hasTarget = input.target !== undefined && input.target !== null;
  const hasUrl = typeof input.url === 'string' && input.url.length > 0;
  if (hasTarget && hasUrl) {
    throw new ActionError('Give at most one of "target" (a thing to click) or "url".', 'INVALID_INPUT');
  }
  return hasTarget ? 'click' : hasUrl ? 'url' : 'next';
}
