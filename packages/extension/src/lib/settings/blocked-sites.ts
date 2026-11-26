/**
 * The blocked-sites list: patterns of pages Vexdom never reads or acts on.
 *
 * It lives in this browser's `storage.local` and nowhere else. No socket frame carries it,
 * so neither the daemon nor the agent it spawns can see or loosen it — the extension checks
 * it before every page action, whoever asked for that action.
 */

import {
  blockedSitesBlockedBy,
  blockedSitesDescribe,
  blockedSitesParse,
  blockedSitesPatternsFor,
  blockedSitesSubject,
  blockedSitesValid,
  jsonText,
} from '@/lib/bridge/pure';

export const BLOCKED_SITES_KEY = 'vexdom/blockedSites';

export const BLOCKED_SITES_LIMITS = { entries: 500, length: 200 } as const;

export const SITE_BLOCKED = 'SITE_BLOCKED';

export const SITE_BLOCKED_MESSAGE =
  'This site is on the user’s Blocked sites list, so Vexdom will not read or act on it. ' +
  'Do not retry or reach it another way (another URL, a search, another tab) — tell the user, or carry on with work that does not need it.';

export const LIST_UNREADABLE_MESSAGE =
  'Vexdom could not read the user’s Blocked sites list, so it is refusing page actions until it can. ' +
  'Ask the user to open Settings → Blocked sites.';

/**
 * A compiled list: its JSON text. The parsing and matching are `vexdom-pure`'s `blocked_sites`
 * through ext-core (P9), which compiles a list once per change; calls need `loadCore()` first.
 */
export interface BlockedList {
  readonly text: string;
}

export const NO_BLOCKED_SITES: BlockedList = { text: '[]' };

export type ParsedPattern =
  | { readonly ok: true; readonly pattern: string; readonly source: string }
  | { readonly ok: false; readonly reason: string };

export function parseBlockedPattern(entry: string): ParsedPattern {
  return JSON.parse(blockedSitesParse(entry)) as ParsedPattern;
}

/** `null` when the stored value is not a list at all — the caller must refuse, not guess. */
export function compileBlockedSites(raw: unknown): BlockedList | null {
  const text = raw === undefined ? '[]' : jsonText(raw);
  return blockedSitesValid(text) ? { text } : null;
}

/** The regex sources of the patterns that could match somewhere on this origin — all a page on it ever needs to be told. */
export function patternsFor(origin: string, list: BlockedList): string[] {
  return blockedSitesPatternsFor(list.text, origin);
}

export function blockedBy(url: string | undefined, list: BlockedList): string | null {
  return url ? blockedSitesBlockedBy(list.text, url) : null;
}

/** What a pattern is matched against: host, port and path with the query — never the hash. */
export function subjectOf(url: string | undefined): string | null {
  return url ? blockedSitesSubject(url) : null;
}

export function describePattern(pattern: string): string {
  return blockedSitesDescribe(pattern);
}
