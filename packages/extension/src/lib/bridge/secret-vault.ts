/**
 * The one place a plaintext secret is kept.
 *
 * It lives in the extension, in `storage.session`, and nowhere else. Not in a
 * transcript, not in `storage.local`, and never across the socket — so the daemon, which
 * spawns an agent CLI and serves MCP clients, has no credential to lose. Session storage
 * is the right area for exactly one reason: the browser empties it on restart, so a
 * password read on Tuesday is not still here on Wednesday.
 *
 * `tag` is minted once per browser session and is the reason a page cannot help itself
 * to what is in here. Handles are public and a page can copy one, but resolving one
 * requires the tag, which is never rendered into page text, a tool result or a
 * transcript.
 *
 * Sealing, release and the vault's rules (TTL, reuse, the release fields) are `vexdom-pure`'s
 * `seal` module through ext-core (P9); this file keeps the storage and the lock.
 */

import { browser } from 'wxt/browser';
import { jsonText, loadCore, vaultRelease, vaultSeal } from './pure';

export type SecretKind = 'api-key' | 'token' | 'jwt' | 'password' | 'cookie' | 'private-key' | 'card' | 'secret';

export const REFUSED_MESSAGE =
  'A sealed secret can only be released into page.fillInput “value”, page.typeText or page.insertText “text”, or page.cookies “value”. ' +
  'Vexdom swaps the placeholder for the real value at the moment it reaches the page, so the secret is never ' +
  'yours to move: pass the placeholder through unchanged and let the field it belongs in do it.';

export const EXPIRED_MESSAGE =
  'That sealed secret is no longer held — it expired, or it was read in an earlier browser session. ' +
  'Read the value from the page again, or ask the user for it.';

const VAULT_KEY = 'vexdom/secrets';

interface Vault {
  tag: string;
  entries: unknown[];
}

export interface Release {
  readonly input: unknown;
  readonly released: number;
  /** Handles sitting in a field that may not carry one. */
  readonly refused: number;
  /** Ours by shape but not by tag, or gone from the vault. */
  readonly unresolved: number;
}

let queue: Promise<unknown> = Promise.resolve();

/**
 * JSON text the Rust side can parse: a lone surrogate (half an emoji cut by a slice, or planted by a
 * page) is U+FFFD, since serde_json refuses the `\ud800` escape `JSON.stringify` writes for it.
 */
function wellFormedJson(value: unknown): string {
  return JSON.stringify(value, (_key, nested: unknown) => (typeof nested === 'string' ? nested.toWellFormed() : nested)) ?? 'null';
}

function locked<T>(task: () => Promise<T>): Promise<T> {
  const run = queue.then(task, task);
  queue = run.catch(() => undefined);
  return run;
}

export function sealForPage<T>(value: T, origin?: string): Promise<{ value: T }> {
  return locked(async () => {
    await loadCore();
    const vault = await readVault();
    const sealed = vaultSeal(jsonText(vault), wellFormedJson(value), origin ?? null, Date.now(), hex(8));
    if (sealed.vault !== null) await writeVault(JSON.parse(sealed.vault) as Vault);
    return { value: (value === undefined ? undefined : JSON.parse(sealed.value)) as T };
  });
}

export function releaseForAction(action: string, input: unknown): Promise<Release> {
  return locked(async () => {
    await loadCore();
    const vault = await readVault();
    const release = vaultRelease(jsonText(vault), action, wellFormedJson(input), Date.now());
    if (release.vault !== null) await writeVault(JSON.parse(release.vault) as Vault);
    return {
      input: input === undefined ? undefined : JSON.parse(release.input),
      released: release.released,
      refused: release.refused,
      unresolved: release.unresolved,
    };
  });
}

export function forgetSecrets(): Promise<void> {
  return locked(() => browser.storage.session.remove(VAULT_KEY));
}

async function readVault(): Promise<Vault> {
  const stored = await browser.storage.session.get(VAULT_KEY).catch(() => ({}) as Record<string, unknown>);
  const held = stored[VAULT_KEY] as Partial<Vault> | undefined;
  if (held?.tag && Array.isArray(held.entries)) return { tag: held.tag, entries: held.entries };
  const fresh: Vault = { tag: hex(8), entries: [] };
  await writeVault(fresh);
  return fresh;
}

function writeVault(vault: Vault): Promise<void> {
  return browser.storage.session.set({ [VAULT_KEY]: vault }).catch(() => undefined);
}

const hex = (bytes: number) =>
  [...crypto.getRandomValues(new Uint8Array(bytes))].map((byte) => byte.toString(16).padStart(2, '0')).join('');
