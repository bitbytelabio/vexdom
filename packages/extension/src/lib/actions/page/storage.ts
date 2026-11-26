import { z } from 'zod';
import { ActionError, defineAction } from '../core';

export const storage = defineAction({
  name: 'page.storage',
  group: 'storage',
  description:
    'Read or change the page’s localStorage or sessionStorage (of the frame in focus): get every entry or one key, set one, remove one, or clear the area.',
  input: z.object({
    area: z.enum(['local', 'session']).default('local').describe('localStorage or sessionStorage'),
    op: z.enum(['get', 'set', 'remove', 'clear']).default('get').describe('What to do'),
    key: z.string().optional().describe('The key; required for set and remove'),
    value: z.string().optional().describe('The value to set'),
  }),
  execute({ area, op, key, value }) {
    const store = area === 'local' ? window.localStorage : window.sessionStorage;
    switch (op) {
      case 'get':
        if (key !== undefined) return { value: store.getItem(key) };
        return { entries: Object.fromEntries(Array.from({ length: store.length }, (_, at) => store.key(at)!).map((name) => [name, store.getItem(name)])) };
      case 'set':
        if (key === undefined || value === undefined) throw new ActionError('"set" needs a "key" and a "value"', 'INVALID_INPUT');
        store.setItem(key, value);
        return { set: key };
      case 'remove':
        if (key === undefined) throw new ActionError('"remove" needs a "key"', 'INVALID_INPUT');
        store.removeItem(key);
        return { removed: key };
      case 'clear': {
        const cleared = store.length;
        store.clear();
        return { cleared };
      }
    }
  },
});
