import { beforeEach, describe, expect, it } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { forgetSeen, OUTSIDE_CALLER, rememberSeen, seenHost } from './seen-sites';

beforeEach(() => fakeBrowser.reset());

describe('seen sites', () => {
  it('holds the site a tab was last looked at on, for the caller that looked', async () => {
    await rememberSeen(3, 'a.com', 's1');
    expect(await seenHost(3, 's1')).toBe('a.com');
    expect(await seenHost(3, OUTSIDE_CALLER)).toBeUndefined();
  });

  it('lets go of a tab looked at on a browser page', async () => {
    await rememberSeen(3, 'a.com', 's1');
    await rememberSeen(3, undefined, 's1');
    expect(await seenHost(3, 's1')).toBeUndefined();
  });

  it('forgets a closed tab for every caller, and only that tab', async () => {
    await rememberSeen(3, 'a.com', 's1');
    await rememberSeen(3, 'a.com', OUTSIDE_CALLER);
    await rememberSeen(13, 'b.com', 's1');
    await forgetSeen(3);
    expect(await seenHost(3, 's1')).toBeUndefined();
    expect(await seenHost(3, OUTSIDE_CALLER)).toBeUndefined();
    expect(await seenHost(13, 's1')).toBe('b.com');
  });
});
