/**
 * `page.dialog`: the JavaScript dialogs that block a tab, through `Page.javascriptDialogOpening`
 * and `Page.handleJavaScriptDialog` (agent-browser `handle_dialog`, `start_dialog_handler`). The
 * events only arrive while attached, so the `dialog` lease is held while auto-answering is on;
 * a status asked of an unwatched tab probes whether the page is blocked.
 */
import { z } from 'zod';
import { dialog } from '@/lib/actions/page/dialog';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, onDebuggerEvent, onLost, release, send, settle, withDebugger } from './cdp';

const OWNER = 'dialog';
const FIREFOX_HINT = 'Dialogs are answered through Chrome’s debugger, which Firefox does not offer to extensions.';
const PROBE_MS = 600;

interface Pending {
  type: string;
  message: string;
  defaultPrompt?: string;
}

interface Watch {
  pending?: Pending;
  alert: boolean;
  beforeunload: boolean;
}

const watched = new Map<number, Watch>();

export async function dialogInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = dialog.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { op, promptText, alert, beforeunload } = parsed.data;

  if (op === 'auto') {
    if (!alert && !beforeunload) {
      if (watched.delete(tabId)) await release(tabId, OWNER);
      return success({ auto: false });
    }
    const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
    if (!acquired.ok) return failure(acquired.code, acquired.message);
    await send({ tabId }, 'Page.enable').catch(() => undefined);
    watched.set(tabId, { ...watched.get(tabId), alert, beforeunload });
    return success({ auto: true, alert, beforeunload });
  }

  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    await send(session, 'Page.enable').catch(() => undefined);
    // Chrome replays an open dialog to a session that enables Page; give it a moment to land.
    await settle(50);
    const known = watched.get(tabId)?.pending ?? opened.get(tabId);
    if (op === 'status') {
      if (known) return success({ hasDialog: true, ...known });
      const blocked = await Promise.race([
        send(session, 'Runtime.evaluate', { expression: '1', timeout: 500 }).then(
          () => false,
          () => false,
        ),
        settle(PROBE_MS).then(() => true),
      ]);
      return success(blocked ? { hasDialog: true, type: 'unknown' } : { hasDialog: false });
    }
    const accept = op === 'accept';
    try {
      await send(session, 'Page.handleJavaScriptDialog', { accept, ...(promptText !== undefined ? { promptText } : {}) });
    } catch (error) {
      clear(tabId);
      const message = error instanceof Error ? error.message : String(error);
      return /no dialog/i.test(message) ? failure('NO_DIALOG', 'No dialog is open on this tab.') : failure('ACTION_FAILED', message);
    }
    clear(tabId);
    return success({ handled: true, accepted: accept });
  });
}

/** Dialogs seen on tabs no auto policy watches, while some other lease kept the debugger there. */
const opened = new Map<number, Pending>();

function clear(tabId: number): void {
  opened.delete(tabId);
  const watch = watched.get(tabId);
  if (watch) watch.pending = undefined;
}

export function serveDialogs(): void {
  if (import.meta.env.FIREFOX) return;
  onDebuggerEvent((source, method, params) => {
    if (source.sessionId || source.tabId == null) return;
    const tabId = source.tabId;
    if (method === 'Page.javascriptDialogClosed') {
      clear(tabId);
      return;
    }
    if (method !== 'Page.javascriptDialogOpening') return;
    const event = (params ?? {}) as { type?: string; message?: string; defaultPrompt?: string };
    const pending: Pending = {
      type: event.type ?? 'alert',
      message: event.message ?? '',
      ...(event.defaultPrompt ? { defaultPrompt: event.defaultPrompt } : {}),
    };
    const watch = watched.get(tabId);
    if (watch && ((pending.type === 'alert' && watch.alert) || (pending.type === 'beforeunload' && watch.beforeunload))) {
      void send(source, 'Page.handleJavaScriptDialog', { accept: true }).catch(() => undefined);
      return;
    }
    if (watch) watch.pending = pending;
    else opened.set(tabId, pending);
  });
  onLost((tabId) => {
    watched.delete(tabId);
    opened.delete(tabId);
  });
}
