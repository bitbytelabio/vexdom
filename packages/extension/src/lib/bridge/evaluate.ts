import { browser } from 'wxt/browser';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { autoRunReady } from './auto-run';
import { focusedFrame } from './frame-focus';

export const USER_SCRIPTS_HINT = 'Turn on “Allow User Scripts” for Vexdom in chrome://extensions → Details, then retry.';

/** What the wrapper hands back: a JSON-safe value (or the token of one JSON cannot hold), or the page's exception. */
type Outcome = { value: unknown; unserializable?: string } | { thrown: string };

/**
 * The code as an async function body, settled inside the page so only JSON crosses: the value is
 * `JSON.parse(JSON.stringify(v))`; `undefined`, `NaN`, `±Infinity`, `-0` and a bigint come back as
 * `{ value: null, unserializable: "<token>" }`; another value JSON cannot carry is `String(v)`, and
 * an exception is its message. `asText` settles to that outcome as JSON text, for Firefox's page `eval`.
 */
function wrap(code: string, form: 'expression' | 'statements', asText = false): string {
  const body = form === 'expression' ? `(async () => (\n${code}\n))()` : `(async () => {\n${code}\n})()`;
  const out = asText ? 'JSON.stringify' : '';
  return `${body}.then(
  (v) => {
    const u = v === undefined ? 'undefined' : typeof v === 'bigint' ? v + 'n' : typeof v === 'number' && !Number.isFinite(v) ? String(v) : Object.is(v, -0) ? '-0' : null;
    if (u !== null) return ${out}({ value: null, unserializable: u });
    try { const json = JSON.stringify(v); return ${out}({ value: json === undefined ? String(v) : JSON.parse(json) }); } catch { return ${out}({ value: String(v) }); } },
  (e) => ${out}({ thrown: e instanceof Error ? e.name + ': ' + e.message : String(e) }),
)`;
}

/**
 * Firefox runs `tabs.executeScript` code in the content-script sandbox, which holds the extension's
 * APIs, so the agent's code never runs there: the sandbox hands it to the page's own `eval` (page
 * globals, no extension APIs, the page's CSP) and takes back only JSON text. A parse error is the
 * one thing `eval` throws synchronously.
 */
function inPageWorld(pageSource: string): string {
  return `(() => {
  let settled;
  try {
    settled = window.wrappedJSObject.eval(${JSON.stringify(pageSource)});
  } catch (error) {
    const named = String(error && error.name) + ': ' + String(error && error.message);
    return error && error.name === 'SyntaxError' ? { syntax: named } : { thrown: named };
  }
  return new Promise((resolve) => {
    settled.then(
      exportFunction((json) => resolve(JSON.parse(String(json))), window),
      exportFunction(() => resolve({ thrown: 'The page code did not settle' }), window),
    );
  });
})()`;
}

const isSyntaxError = (text: string | undefined) => !!text && /SyntaxError/.test(text);

interface Injection {
  result?: unknown;
  error?: unknown;
}

type FirefoxTabs = {
  executeScript(tabId: number, details: { code: string; frameId?: number }): Promise<unknown[]>;
};

const NOT_PARSED = 'SyntaxError: the code does not parse as an expression or as statements';

/** The wrapper's own reply; anything else means the script never ran, which is how a parse error shows. */
function outcomeOf(result: unknown): Outcome | { syntax: string } {
  if (result && typeof result === 'object' && ('value' in result || 'thrown' in result)) return result as Outcome;
  return { syntax: NOT_PARSED };
}

/** Runs one wrapped script; a parse failure comes back as `{ syntax }` so the caller can try the other form. */
async function inject(tabId: number, frameId: number, script: string): Promise<Outcome | { syntax: string }> {
  try {
    if (import.meta.env.FIREFOX) {
      const [result] = await (browser.tabs as unknown as FirefoxTabs).executeScript(tabId, { code: script, frameId });
      const reply = result as Outcome | { syntax: string } | undefined;
      return reply && typeof reply === 'object' ? reply : { thrown: 'The page code returned nothing' };
    }
    const [injection] = (await browser.userScripts.execute({
      target: { tabId, frameIds: [frameId] },
      world: 'MAIN',
      injectImmediately: true,
      js: [{ code: script }],
    })) as Injection[];
    const error = injection?.error === undefined ? undefined : String(injection.error);
    if (error !== undefined) return isSyntaxError(error) ? { syntax: error } : { thrown: error };
    return outcomeOf(injection?.result);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (isSyntaxError(message)) return { syntax: message };
    throw error;
  }
}

export function evaluateReady(): boolean {
  return import.meta.env.FIREFOX || (autoRunReady() && typeof browser.userScripts?.execute === 'function');
}

/** Evaluates `code` in the focused frame: as an expression first, then as statements if that does not parse. */
export async function evaluateInTab(tabId: number, code: string, timeoutMs: number): Promise<ActionResult> {
  if (!evaluateReady()) return failure('USER_SCRIPTS_DISABLED', USER_SCRIPTS_HINT);
  const frameId = await focusedFrame(tabId);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<ActionResult>((resolve) => {
    timer = setTimeout(() => resolve(failure('TIMEOUT', `The code did not finish within ${timeoutMs} ms.`)), timeoutMs);
  });
  const run = (async (): Promise<ActionResult> => {
    const script = (form: 'expression' | 'statements') =>
      import.meta.env.FIREFOX ? inPageWorld(wrap(code, form, true)) : wrap(code, form);
    let outcome = await inject(tabId, frameId, script('expression'));
    if ('syntax' in outcome) outcome = await inject(tabId, frameId, script('statements'));
    if ('syntax' in outcome) return failure('EVAL_FAILED', outcome.syntax);
    if ('thrown' in outcome) return failure('EVAL_FAILED', outcome.thrown);
    return success(
      outcome.unserializable === undefined ? { value: outcome.value } : { value: null, unserializable: outcome.unserializable },
    );
  })().catch((error: unknown) => failure('EVAL_FAILED', error instanceof Error ? error.message : String(error)));
  try {
    return await Promise.race([run, timeout]);
  } finally {
    clearTimeout(timer);
  }
}
