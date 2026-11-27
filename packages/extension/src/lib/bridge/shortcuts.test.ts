import { beforeEach, describe, expect, it } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { listShortcuts, removeShortcut, saveShortcut } from './shortcuts';

beforeEach(() => fakeBrowser.reset());

describe('saveShortcut', () => {
  it('takes lowercase names only', async () => {
    expect(await saveShortcut({ name: 'Sum', prompt: 'Summarise this page' })).toEqual({
      ok: false,
      error: 'Use lowercase letters, digits and dashes, up to 32.',
    });
  });

  it('keeps the built-in commands', async () => {
    expect(await saveShortcut({ name: 'tabs', prompt: 'x' })).toEqual({
      ok: false,
      error: '/tabs is a built-in command.',
    });
  });

  it('refuses a name already taken', async () => {
    await saveShortcut({ name: 'sum', prompt: 'Summarise this page' });
    expect(await saveShortcut({ name: 'sum', prompt: 'Other' })).toEqual({ ok: false, error: '/sum already exists.' });
  });

  it('updates one in place by id, and removes it', async () => {
    const saved = await saveShortcut({ name: 'sum', prompt: 'Summarise this page' });
    if (!saved.ok) throw new Error(saved.error);
    expect(
      await saveShortcut({ id: saved.shortcut.id, name: 'sum', prompt: 'Summarise it in three lines' }),
    ).toMatchObject({ ok: true });
    const all = await listShortcuts();
    expect(all).toHaveLength(1);
    expect(all[0].prompt).toBe('Summarise it in three lines');

    await removeShortcut(saved.shortcut.id);
    expect(await listShortcuts()).toEqual([]);
  });
});
