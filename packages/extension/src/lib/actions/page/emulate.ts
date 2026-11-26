import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const emulate = defineAction({
  name: 'page.emulate',
  chromiumOnly: true,
  group: 'emulation',
  description:
    'Make the tab pretend: a viewport size, a phone or tablet (device), a location, being offline, extra request headers, HTTP basic credentials, print media or a colour scheme. Each section given is applied; reset: true puts everything back.',
  input: z.object({
    viewport: z
      .object({
        width: z.number().int().min(1).max(10_000),
        height: z.number().int().min(1).max(10_000),
        deviceScaleFactor: z.number().min(0.1).max(10).default(1),
        mobile: z.boolean().default(false),
      })
      .optional(),
    device: z.string().optional().describe('A device name such as "iPhone 15", "Pixel 9" or "iPad Pro"'),
    geolocation: z
      .object({
        latitude: z.number().min(-90).max(90),
        longitude: z.number().min(-180).max(180),
        accuracy: z.number().min(0).default(1),
      })
      .optional(),
    offline: z.boolean().optional(),
    headers: z.record(z.string(), z.string()).optional().describe('Headers added to every request of the tab'),
    credentials: z
      .object({ username: z.string(), password: z.string() })
      .optional()
      .describe('HTTP basic credentials sent with every request of the tab'),
    media: z
      .object({
        type: z.enum(['screen', 'print']).optional(),
        colorScheme: z.enum(['light', 'dark', 'no-preference']).optional(),
        reducedMotion: z.enum(['reduce', 'no-preference']).optional(),
      })
      .optional(),
    reset: z.boolean().default(false).describe('Undo every emulation on the tab'),
  }),
  execute() {
    throw new ActionError('page.emulate is resolved by the Vexdom extension, not in the page', 'UNSUPPORTED');
  },
});
