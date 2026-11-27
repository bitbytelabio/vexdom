import { describe, expect, it } from 'vitest';
import { markRosterSent, rosterChanged } from './roster';

const tab = (tabId: number) => ({ tabId, title: `Tab ${tabId}`, host: 'a.com' });

describe('rosterChanged', () => {
  it('says nothing to a Bridge that was never sent a roster', () => {
    expect(rosterChanged('never', [tab(1)])).toBe(false);
  });

  it('reports a change once, then not again', () => {
    markRosterSent('s', []);
    expect(rosterChanged('s', [])).toBe(false);
    expect(rosterChanged('s', [tab(1)])).toBe(true);
    expect(rosterChanged('s', [tab(1)])).toBe(false);
  });
});
