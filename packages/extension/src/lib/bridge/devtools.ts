/**
 * `page.vitals`, `page.react` and `page.a11yAudit` (ADR-030). The page scripts are agent-browser's
 * (`vendor/react/scripts.ts`) and React DevTools' own hook (`vendor/react/installHook.js`, MIT);
 * each answers raw JSON, which the Bridge turns into agent-browser's reports. axe-core arrives
 * from the Bridge in the input (`axeSource`), fetched there at first use, never bundled.
 */
import { z } from 'zod';
import { a11yAudit } from '@/lib/actions/page/a11y-audit';
import { react } from '@/lib/actions/page/react';
import { vitals } from '@/lib/actions/page/vitals';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, onLost, send, settle, withDebugger, type DebuggerSession } from './cdp';
import { watchForLoad } from './tabs';
import INSTALL_HOOK_JS from './vendor/react/installHook.js?raw';
import {
  PICK_REACT_RENDERER,
  RENDERS_INIT,
  RENDERS_STOP,
  SUSPENSE_WALK,
  TREE_INSPECT,
  TREE_SNAPSHOT,
  VITALS_INIT,
  VITALS_READ,
} from './vendor/react/scripts';

const FIREFOX_HINT = 'This needs Chrome’s debugger, which Firefox does not offer to extensions.';
const VITALS_SETTLE_MS = 3000;

interface Evaluated {
  result?: { value?: unknown };
  exceptionDetails?: { text?: string; exception?: { description?: string } };
}

async function evaluate(session: DebuggerSession, expression: string, awaitPromise = false): Promise<unknown> {
  const evaluated = await send<Evaluated>(session, 'Runtime.evaluate', { expression, returnByValue: true, awaitPromise });
  if (evaluated.exceptionDetails) {
    throw new Error(evaluated.exceptionDetails.exception?.description ?? evaluated.exceptionDetails.text ?? 'The page script threw');
  }
  return evaluated.result?.value;
}

/** The scripts answer a JSON string; anything else is passed on as is. */
function parsedValue(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

async function reloadAndWait(tabId: number, session: DebuggerSession): Promise<void> {
  const loaded = watchForLoad(tabId);
  await send(session, 'Page.reload');
  await loaded.settled;
}

export function vitalsInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = vitals.input.safeParse(input ?? {});
  if (!parsed.success) return Promise.resolve(failure('INVALID_INPUT', z.prettifyError(parsed.error)));
  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    const added = await send<{ identifier: string }>(session, 'Page.addScriptToEvaluateOnNewDocument', { source: VITALS_INIT });
    try {
      await evaluate(session, VITALS_INIT);
      if (parsed.data.reload) {
        await reloadAndWait(tabId, session);
        await settle(VITALS_SETTLE_MS);
      }
      const url = await evaluate(session, 'location.href');
      const raw = parsedValue(await evaluate(session, VITALS_READ));
      return success({ url, raw });
    } finally {
      await send(session, 'Page.removeScriptToEvaluateOnNewDocument', { identifier: added.identifier }).catch(() => undefined);
    }
  });
}

/** Tabs where the DevTools hook is installed for every load: the `react` lease keeps it there. */
const hooked = new Set<number>();

export async function reactInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = react.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { op, fiberId } = parsed.data;

  if (op === 'install') {
    const acquired = await acquire(tabId, 'react', FIREFOX_HINT);
    if (!acquired.ok) return failure(acquired.code, acquired.message);
    try {
      if (!hooked.has(tabId)) {
        await send(acquired.lease.session, 'Page.addScriptToEvaluateOnNewDocument', { source: INSTALL_HOOK_JS });
        hooked.add(tabId);
      }
      await reloadAndWait(tabId, acquired.lease.session);
    } catch (error) {
      return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
    }
    return success({ installed: true });
  }

  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    if (!(await evaluate(session, '!!window.__REACT_DEVTOOLS_GLOBAL_HOOK__'))) {
      return failure('REACT_HOOK_MISSING', 'run react install (reloads the tab) first');
    }
    const withRenderer = (script: string) => script.replaceAll('{{PICK_RI}}', PICK_REACT_RENDERER);
    switch (op) {
      case 'tree':
        return success({ op, raw: parsedValue(await evaluate(session, withRenderer(TREE_SNAPSHOT))) });
      case 'inspect':
        if (fiberId === undefined) return failure('INVALID_INPUT', 'inspect needs the component id as "fiberId".');
        return success({
          op,
          raw: parsedValue(await evaluate(session, withRenderer(TREE_INSPECT).replaceAll('{{ID}}', String(fiberId)))),
        });
      case 'rendersStart': {
        const added = await send<{ identifier: string }>(session, 'Page.addScriptToEvaluateOnNewDocument', { source: RENDERS_INIT });
        await evaluate(session, RENDERS_INIT);
        return success({
          recording: true,
          identifier: added.identifier,
          message: 'recording renders - interact with the page, then run `react renders stop`',
        });
      }
      case 'rendersStop':
        return success({ op, raw: parsedValue(await evaluate(session, RENDERS_STOP)) });
      default:
        return success({ op, raw: parsedValue(await evaluate(session, withRenderer(SUSPENSE_WALK))) });
    }
  });
}

export function a11yInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = a11yAudit.input.safeParse(input ?? {});
  if (!parsed.success) return Promise.resolve(failure('INVALID_INPUT', z.prettifyError(parsed.error)));
  const { axeSource, tags, selector } = parsed.data;
  if (!axeSource) {
    return Promise.resolve(failure('AXE_UNAVAILABLE', 'page.a11yAudit is run through the Vexdom Bridge, which provides axe-core.'));
  }
  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    await evaluate(session, axeSource);
    const options = tags?.length ? { runOnly: { type: 'tag', values: tags } } : {};
    const expression = `(async () => {
      const context = ${selector ? `document.querySelector(${JSON.stringify(selector)}) ?? document` : 'document'};
      const results = await axe.run(context, ${JSON.stringify(options)});
      const skippedFrames = [...document.querySelectorAll('iframe,frame')].filter((frame) => {
        try { return !frame.contentDocument; } catch { return true; }
      }).length;
      return {
        url: location.href,
        violations: results.violations.map((rule) => ({
          id: rule.id,
          impact: rule.impact,
          help: rule.help,
          helpUrl: rule.helpUrl,
          count: rule.nodes.length,
          targets: rule.nodes.slice(0, 5).map((node) => node.target.join(' ')),
        })),
        passes: results.passes.length,
        incomplete: results.incomplete.length,
        inapplicable: results.inapplicable.length,
        skippedFrames,
      };
    })()`;
    return success(await evaluate(session, expression, true));
  });
}

export function serveDevtools(): void {
  onLost((tabId) => hooked.delete(tabId));
}
