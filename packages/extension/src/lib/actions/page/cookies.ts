import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const cookies = defineAction({
  name: 'page.cookies',
  group: 'storage',
  description:
    'Read, set or clear the browser’s cookies for a URL (the tab’s, unless "url" is given), including HttpOnly ones. Credential-shaped values come back sealed.',
  input: z.object({
    op: z.enum(['get', 'set', 'clear']).default('get').describe('What to do'),
    url: z.string().optional().describe('The URL the cookies belong to; defaults to the tab’s'),
    name: z.string().optional().describe('One cookie by name; required for set'),
    value: z.string().optional().describe('The value to set'),
    domain: z.string().optional().describe('Domain for set; leave out for a host-only cookie'),
    path: z.string().optional().describe('Path for set'),
    secure: z.boolean().optional().describe('Secure flag for set'),
    httpOnly: z.boolean().optional().describe('HttpOnly flag for set'),
    sameSite: z.enum(['strict', 'lax', 'no_restriction']).optional().describe('SameSite for set'),
    expires: z.number().optional().describe('Expiry for set, in epoch seconds; leave out for a session cookie'),
  }),
  execute() {
    throw new ActionError('page.cookies is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
