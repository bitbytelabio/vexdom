import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const vitals = defineAction({
  name: 'page.vitals',
  chromiumOnly: true,
  group: 'devtools',
  description:
    'Measure the page’s Core Web Vitals — TTFB, LCP, CLS, FCP, INP — and React hydration timing when the page uses a profiling build. reload: true reloads first so the whole load is measured.',
  input: z.object({
    reload: z.boolean().default(false).describe('Reload the page and measure its load'),
  }),
  execute() {
    throw new ActionError('page.vitals is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
