import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const routeNetwork = defineAction({
  name: 'page.routeNetwork',
  chromiumOnly: true,
  group: 'network',
  description:
    'Answer or block the tab’s requests whose URL matches a pattern ("*" is a wildcard; without one the URL only has to contain it): abort them, or reply with a mocked status, body and headers. Routes apply in the order they were added until page.unrouteNetwork removes them.',
  input: z.object({
    pattern: z.string().min(1).describe('URL pattern, e.g. "*api/items*" or "/api/"'),
    abort: z.boolean().default(false).describe('Fail matching requests as if the network refused them'),
    response: z
      .object({
        status: z.number().int().min(100).max(599).optional().describe('Status code, 200 by default'),
        body: z.string().optional().describe('Response body; JSON gets an application/json content type'),
        contentType: z.string().optional(),
        headers: z.record(z.string(), z.string()).optional(),
        allowOrigin: z.string().optional().describe('The one page origin allowed to read the mock across origins'),
      })
      .optional()
      .describe('The mocked response'),
    resourceTypes: z
      .array(z.string())
      .default([])
      .describe('Only requests of these types, e.g. ["XHR", "Fetch"]; every type when empty'),
  }),
  execute() {
    throw new ActionError('page.routeNetwork is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
