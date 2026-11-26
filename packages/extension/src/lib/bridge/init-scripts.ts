/**
 * `page.initScript`: scripts that run in every document a tab loads, before its own, through
 * `Page.addScriptToEvaluateOnNewDocument`. They live as long as the `init` lease, which is held
 * while a tab has any.
 */
import { z } from 'zod';
import { initScript } from '@/lib/actions/page/init-script';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, detachedFailure, onLost, release, send } from './cdp';

const OWNER = 'init';
const FIREFOX_HINT = 'Init scripts need Chrome’s debugger, which Firefox does not offer to extensions.';

interface Script {
  identifier: string;
  cdp: string;
  bytes: number;
}

const scripts = new Map<number, Script[]>();
const lost = new Map<number, string>();
let counter = 0;

export async function initScriptInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = initScript.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { op, source, identifier } = parsed.data;
  const list = scripts.get(tabId) ?? [];

  if (op === 'list') {
    const gone = lost.get(tabId);
    if (!list.length && gone) {
      lost.delete(tabId);
      return detachedFailure(gone);
    }
    return success({ scripts: list.map(({ identifier: id, bytes }) => ({ identifier: id, bytes })) });
  }

  if (op === 'add') {
    if (!source) return failure('INVALID_INPUT', 'add needs the script as "source".');
    lost.delete(tabId);
    const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
    if (!acquired.ok) return failure(acquired.code, acquired.message);
    try {
      const added = await send<{ identifier: string }>({ tabId }, 'Page.addScriptToEvaluateOnNewDocument', { source });
      const own = `i${++counter}`;
      scripts.set(tabId, [...list, { identifier: own, cdp: added.identifier, bytes: new TextEncoder().encode(source).length }]);
      return success({ added: true, identifier: own });
    } catch (error) {
      if (!list.length) await release(tabId, OWNER);
      return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
    }
  }

  const found = list.find((script) => script.identifier === identifier);
  if (!found) {
    const gone = lost.get(tabId);
    if (gone) {
      lost.delete(tabId);
      return detachedFailure(gone);
    }
    return failure('INVALID_INPUT', `No init script ${identifier ?? '(none given)'} on this tab; list shows them.`);
  }
  await send({ tabId }, 'Page.removeScriptToEvaluateOnNewDocument', { identifier: found.cdp }).catch(() => undefined);
  const left = list.filter((script) => script !== found);
  if (left.length) scripts.set(tabId, left);
  else {
    scripts.delete(tabId);
    await release(tabId, OWNER);
  }
  return success({ removed: true, identifier: found.identifier });
}

export function serveInitScripts(): void {
  onLost((tabId, reason) => {
    if (scripts.delete(tabId)) lost.set(tabId, reason);
  });
}
