import { browser } from 'wxt/browser';
import { MAX_INSTRUCTION_CHARS } from '@/lib/schedules/task';

/** Prompts the user saved to reuse: typed as `/name` in the composer, or copied into a schedule. */
export const SHORTCUTS_KEY = 'vexdom/shortcuts';

export const MAX_SHORTCUTS = 100;

export interface Shortcut {
  id: string;
  name: string;
  prompt: string;
  createdAt: number;
  updatedAt: number;
}

export const SHORTCUT_NAME = /^[a-z0-9][a-z0-9-]{0,31}$/;

/** The composer's own slash commands, which a shortcut cannot hide. */
const BUILT_IN = new Set(['context', 'tabs', 'hands-free', 'remove-tools', 'shortcuts']);

const isShortcut = (value: unknown): value is Shortcut => {
  const entry = value as Partial<Shortcut> | null;
  return typeof entry?.id === 'string' && typeof entry.name === 'string' && typeof entry.prompt === 'string';
};

export async function listShortcuts(): Promise<Shortcut[]> {
  const stored = (await browser.storage.local.get(SHORTCUTS_KEY))[SHORTCUTS_KEY];
  return Array.isArray(stored) ? stored.filter(isShortcut) : [];
}

export async function saveShortcut(draft: {
  id?: string;
  name: string;
  prompt: string;
}): Promise<{ ok: true; shortcut: Shortcut } | { ok: false; error: string }> {
  const name = draft.name.trim();
  const prompt = draft.prompt.trim();
  if (!SHORTCUT_NAME.test(name)) return { ok: false, error: 'Use lowercase letters, digits and dashes, up to 32.' };
  if (BUILT_IN.has(name)) return { ok: false, error: `/${name} is a built-in command.` };
  if (!prompt || prompt.length > MAX_INSTRUCTION_CHARS) {
    return { ok: false, error: `Write the prompt (up to ${MAX_INSTRUCTION_CHARS} characters).` };
  }

  const all = await listShortcuts();
  if (all.some((shortcut) => shortcut.name === name && shortcut.id !== draft.id))
    return { ok: false, error: `/${name} already exists.` };

  const now = Date.now();
  const existing = draft.id ? all.find((shortcut) => shortcut.id === draft.id) : undefined;
  if (!existing && all.length >= MAX_SHORTCUTS) {
    return { ok: false, error: `You have ${MAX_SHORTCUTS} shortcuts; delete one first.` };
  }
  const shortcut: Shortcut = existing
    ? { ...existing, name, prompt, updatedAt: now }
    : { id: crypto.randomUUID(), name, prompt, createdAt: now, updatedAt: now };
  const next = existing ? all.map((entry) => (entry.id === shortcut.id ? shortcut : entry)) : [...all, shortcut];
  await browser.storage.local.set({ [SHORTCUTS_KEY]: next });
  return { ok: true, shortcut };
}

export async function removeShortcut(id: string): Promise<void> {
  const all = await listShortcuts();
  await browser.storage.local.set({ [SHORTCUTS_KEY]: all.filter((shortcut) => shortcut.id !== id) });
}

export function onShortcutsChange(listener: (all: Shortcut[]) => void): () => void {
  const watch = (changes: Record<string, { newValue?: unknown }>) => {
    if (SHORTCUTS_KEY in changes) void listShortcuts().then(listener);
  };
  browser.storage.local.onChanged.addListener(watch);
  return () => browser.storage.local.onChanged.removeListener(watch);
}
