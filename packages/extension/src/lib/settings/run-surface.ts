import { browser } from 'wxt/browser';

/** Each conversation's tabs sit in a tab group of their own. */
export const TAB_GROUPS_KEY = 'vexdom/tabGroups';
/** A notice when a side-panel run finishes, fails or waits while the user is elsewhere. */
export const RUN_NOTICES_KEY = 'vexdom/runNotices';
/** A short chime with each of those notices. */
export const RUN_SOUND_KEY = 'vexdom/runSound';
/** A glowing frame on the pages a run works in. */
export const RUN_GLOW_KEY = 'vexdom/runGlow';

/** All four live in storage.local as booleans; absent means on. */
export const settingOn = (value: unknown): boolean => value !== false;

export async function readSetting(key: string): Promise<boolean> {
  const stored = await browser.storage.local.get(key);
  return settingOn(stored[key]);
}
