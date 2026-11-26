import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const waitFor = defineAction({
  name: 'page.waitFor',
  group: 'navigation',
  description:
    'Wait until every condition given holds: text appears on the page, the URL matches a glob (`**` any path, `*` one segment; no `*` means contains), the page reaches a load state, or a JavaScript expression is truthy. For one element, use page.waitForElement.',
  input: z.object({
    text: z.string().optional().describe('Text the page should contain'),
    url: z.string().optional().describe('Glob or substring the tab URL should match'),
    load: z
      .enum(['load', 'domcontentloaded', 'networkidle'])
      .optional()
      .describe('Load state: "networkidle" means no resource finished for 500 ms'),
    fn: z.string().optional().describe('A JavaScript expression or arrow function evaluated in the page until truthy'),
    timeoutMs: z.number().int().min(100).max(60_000).default(10_000).describe('How long to wait'),
  }),
  execute() {
    throw new ActionError('page.waitFor is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
