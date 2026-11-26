import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { UNDESCRIBED } from './dom';

export const a11yAudit = defineAction({
  name: 'page.a11yAudit',
  chromiumOnly: true,
  group: 'devtools',
  description:
    'Audit the page’s accessibility with axe-core: each rule it breaks, how badly, how many elements and which ones. Frames from other origins are not audited.',
  input: z.object({
    tags: z.array(z.string()).optional().describe('Only these rule tags, e.g. ["wcag2a", "wcag2aa"]'),
    selector: z.string().optional().describe('Audit only this part of the page'),
    axeSource: z.string().optional().meta({ [UNDESCRIBED]: true }),
  }),
  execute() {
    throw new ActionError('page.a11yAudit is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
