/**
 * Reading and calling the tools a site registers through WebMCP.
 *
 * `document.modelContext` (and its older `navigator.modelContext` alias) lives in the
 * page's main world — a native implementation is visible from the isolated world, but a
 * polyfilled one is not, so both paths go through `scripting.executeScript` with
 * `world: 'MAIN'`. The injected functions are serialized, which is why each one is
 * self-contained, never throws, and hands back plain JSON either way.
 */

import { browser } from 'wxt/browser';
import { z } from 'zod';
import { invokeInTab } from '@/lib/actions/client';
import { callSiteTool } from '@/lib/actions/page/call-site-tool';
import { cancelSiteTool } from '@/lib/actions/page/cancel-site-tool';
import { listSiteTools } from '@/lib/actions/page/list-site-tools';
import { siteToolResult } from '@/lib/actions/page/site-tool-result';
import { getPageInfo } from '@/lib/actions/page/get-page-info';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { captchaOnPage } from './captcha-probe';
import { originOf } from './code-toolkit';
import { focusedFrame, framePath } from './frame-focus';
import { focusOf } from './frames';

const TIMEOUT_MARKER = '⟪vexdom:site-tool-timeout⟫';

interface SiteToolDescriptor {
  name: string;
  description?: string;
  inputSchema?: unknown;
  annotations?: unknown;
}

type ReadReply =
  | { ok: true; api: string | null; origin: string; tools: SiteToolDescriptor[] }
  | { ok: false; error: string };

type CallReply =
  | { ok: true; value: unknown }
  | { ok: false; code: string; error: string; tools?: string[] };

async function readRegisteredTools(withSchemas: boolean): Promise<ReadReply> {
  try {
    const fromDocument = (document as { modelContext?: Record<string, unknown> }).modelContext;
    const fromNavigator = (navigator as { modelContext?: Record<string, unknown> }).modelContext;
    const host = fromDocument ?? fromNavigator;
    const origin = location.origin;
    if (!host) return { ok: true, api: null, origin, tools: [] };

    const api = fromDocument ? 'document.modelContext' : 'navigator.modelContext';
    const enumerate = host.getTools ?? host.listTools;
    if (typeof enumerate !== 'function') return { ok: true, api, origin, tools: [] };

    const raw = await Promise.resolve((enumerate as () => unknown).call(host));
    const list = Array.isArray(raw)
      ? raw
      : Array.isArray((raw as { tools?: unknown } | null)?.tools)
        ? ((raw as { tools: unknown[] }).tools)
        : [];

    const safe = (value: unknown) => {
      try {
        return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
      } catch {
        return undefined;
      }
    };

    const tools = list
      .filter((tool): tool is Record<string, unknown> => !!tool && typeof (tool as { name?: unknown }).name === 'string')
      .map((tool) => ({
        name: String(tool.name),
        description:
          typeof tool.description === 'string' ? tool.description.slice(0, withSchemas ? 500 : 140) : undefined,
        ...(withSchemas
          ? {
              inputSchema: safe(tool.inputSchema ?? tool.input_schema ?? tool.parameters),
              annotations: safe(tool.annotations),
            }
          : {}),
      }));
    return { ok: true, api, origin, tools };
  } catch (error) {
    return { ok: false, error: String((error as { message?: unknown } | null)?.message ?? error) };
  }
}

async function callRegisteredTool(
  name: string,
  args: Record<string, unknown>,
  timeoutMs: number,
  timeoutMarker: string,
): Promise<CallReply> {
  try {
    const fromDocument = (document as { modelContext?: Record<string, unknown> }).modelContext;
    const fromNavigator = (navigator as { modelContext?: Record<string, unknown> }).modelContext;
    const host = fromDocument ?? fromNavigator;
    if (!host) return { ok: false, code: 'NO_SITE_TOOLS', error: 'This site registers no WebMCP tools.' };

    const enumerate = host.getTools ?? host.listTools;
    if (typeof enumerate === 'function') {
      const raw = await Promise.resolve((enumerate as () => unknown).call(host));
      const list = Array.isArray(raw)
        ? raw
        : Array.isArray((raw as { tools?: unknown } | null)?.tools)
          ? ((raw as { tools: unknown[] }).tools)
          : [];
      const names = list
        .map((tool) => (tool as { name?: unknown } | null)?.name)
        .filter((candidate): candidate is string => typeof candidate === 'string');
      if (names.length && !names.includes(name)) {
        return {
          ok: false,
          code: 'SITE_TOOL_NOT_FOUND',
          error: `This site registers no tool named “${name}”.`,
          tools: names,
        };
      }
    }

    const invoke = host.executeTool ?? host.callTool;
    if (typeof invoke !== 'function') {
      return { ok: false, code: 'NO_SITE_TOOLS', error: 'This site’s model context offers no way to execute a tool.' };
    }

    const outcome = await Promise.race([
      Promise.resolve((invoke as (tool: string, input: unknown) => unknown).call(host, name, args)),
      new Promise((resolve) => setTimeout(() => resolve(timeoutMarker), timeoutMs)),
    ]);
    if (outcome === timeoutMarker) {
      return { ok: false, code: 'TIMEOUT', error: `“${name}” did not finish within ${timeoutMs}ms.` };
    }

    try {
      return { ok: true, value: outcome === undefined ? null : JSON.parse(JSON.stringify(outcome)) };
    } catch {
      return { ok: false, code: 'SITE_TOOL_FAILED', error: 'The tool returned something that is not JSON.' };
    }
  } catch (error) {
    return {
      ok: false,
      code: 'SITE_TOOL_FAILED',
      error: String((error as { message?: unknown } | null)?.message ?? error),
    };
  }
}

async function inMainWorld<T>(
  tabId: number,
  func: (...args: never[]) => Promise<T>,
  args: unknown[],
  frameId?: number,
): Promise<T | null> {
  try {
    const results = await browser.scripting.executeScript({
      target: { tabId, frameIds: [frameId ?? (await focusedFrame(tabId))] },
      world: 'MAIN',
      func: func as () => Promise<T>,
      args: args as [],
    });
    return (results[0]?.result as T | undefined) ?? null;
  } catch {
    return null;
  }
}

export async function probeSiteTools(tabId: number) {
  const read = await inMainWorld<ReadReply>(tabId, readRegisteredTools, [false]);
  if (!read?.ok || !read.tools.length) return null;
  return { api: read.api, count: read.tools.length, tools: read.tools };
}

export async function pageInfoWithSiteTools(tabId: number, topUrl: string | undefined, input: unknown): Promise<ActionResult> {
  const [info, siteTools, path, captcha] = await Promise.all([
    invokeInTab(tabId, getPageInfo.name, input),
    probeSiteTools(tabId),
    framePath(tabId),
    captchaOnPage(tabId),
  ]);
  if (!info.ok) return info;
  return success({
    ...(info.data as object),
    ...(captcha ? { captcha } : {}),
    ...(siteTools ? { siteTools } : {}),
    ...(path.length ? focusOf(path, topUrl) : {}),
  });
}

interface FoundTool extends SiteToolDescriptor {
  origin: string;
  frameId: number;
}

/** Every frame's registered tools, each tagged with the origin and frame that registered it. */
async function discoverAll(tabId: number, withSchemas: boolean): Promise<{ api: string | null; tools: FoundTool[] } | null> {
  let results: { frameId: number; result?: unknown }[];
  try {
    results = await browser.scripting.executeScript({
      target: { tabId, allFrames: true },
      world: 'MAIN',
      func: readRegisteredTools as () => Promise<ReadReply>,
      args: [withSchemas] as unknown as [],
    });
  } catch {
    return null;
  }
  let api: string | null = null;
  const tools: FoundTool[] = [];
  for (const { frameId, result } of results) {
    const read = result as ReadReply | undefined;
    if (!read?.ok) continue;
    api ??= read.api;
    tools.push(...read.tools.map((tool) => ({ ...tool, origin: read.origin, frameId })));
  }
  return { api, tools };
}

export async function listSiteToolsInTab(tabId: number, url: string | undefined, input: unknown): Promise<ActionResult> {
  const parsed = listSiteTools.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const pageOrigin = originOf(url);
  if (!pageOrigin) return failure('UNSUPPORTED', 'Site tools can only be read on an http(s) page.');
  const { tool, frameId, withSchemas } = parsed.data;

  const found = await discoverAll(tabId, withSchemas || tool !== undefined);
  if (!found) {
    return failure('UNSUPPORTED', 'Reading site tools needs main-world scripting, which this browser does not allow here.');
  }
  const tools = found.tools.filter(
    (listed) => (frameId === undefined || listed.frameId === frameId) && (tool === undefined || listed.name === tool),
  );
  if (!tools.length) {
    return tool !== undefined
      ? failure('SITE_TOOL_NOT_FOUND', `This page registers no tool named “${tool}”.`)
      : failure('NO_SITE_TOOLS', 'This site registers no WebMCP tools — the ordinary page tools are the way to work here.');
  }
  return success({ pageOrigin, api: found.api, count: tools.length, tools });
}

interface Invocation {
  tool: string;
  frameId?: number;
  status: 'running' | 'done' | 'failed' | 'cancelled';
  startedAt: number;
  returned?: unknown;
  error?: { code: string; message: string };
  settled: Promise<void>;
}

/** agent-browser's `MAX_INVOCATION_HISTORY`: the oldest detached call is forgotten past this. */
const MAX_INVOCATIONS = 128;
const invocations = new Map<string, Invocation>();
let invocationCounter = 0;

async function runSiteTool(
  tabId: number,
  tool: string,
  args: Record<string, unknown>,
  timeoutMs: number,
  frameId: number | undefined,
): Promise<ActionResult> {
  const called = await inMainWorld<CallReply>(tabId, callRegisteredTool, [tool, args, timeoutMs, TIMEOUT_MARKER], frameId);
  if (!called) {
    return failure('UNSUPPORTED', 'Calling site tools needs main-world scripting, which this browser does not allow here.');
  }
  if (!called.ok) {
    const listed = called.tools?.length ? ` It defines: ${called.tools.join(', ')}.` : '';
    return failure(called.code, `${called.error}${listed}`);
  }
  return success({ tool, returned: called.value });
}

export async function callSiteToolInTab(tabId: number, url: string | undefined, input: unknown): Promise<ActionResult> {
  const parsed = callSiteTool.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  if (!originOf(url)) return failure('UNSUPPORTED', 'Site tools can only be called on an http(s) page.');

  const { tool, args, timeoutMs, frameId, detach } = parsed.data;
  if (!detach) return runSiteTool(tabId, tool, args, timeoutMs, frameId);

  const invocationId = `w${++invocationCounter}`;
  const invocation: Invocation = { tool, frameId, status: 'running', startedAt: Date.now(), settled: Promise.resolve() };
  invocation.settled = runSiteTool(tabId, tool, args, timeoutMs, frameId).then((result) => {
    if (invocation.status === 'cancelled') return;
    if (result.ok) {
      invocation.status = 'done';
      invocation.returned = (result.data as { returned?: unknown }).returned;
    } else {
      invocation.status = 'failed';
      invocation.error = result.error;
    }
  });
  invocations.set(invocationId, invocation);
  while (invocations.size > MAX_INVOCATIONS) invocations.delete(invocations.keys().next().value!);
  return success({ invocationId });
}

const GONE = 'the extension restarted or the id expired';

export async function siteToolOutcome(input: unknown): Promise<ActionResult> {
  const parsed = siteToolResult.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { invocationId, waitMs } = parsed.data;
  const invocation = invocations.get(invocationId);
  if (!invocation) return failure('SITE_TOOL_INVOCATION_GONE', `No call ${invocationId}: ${GONE}.`);
  if (invocation.status === 'running' && waitMs > 0) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([invocation.settled, new Promise((resolve) => (timer = setTimeout(resolve, waitMs)))]);
    clearTimeout(timer);
  }
  return success({
    invocationId,
    tool: invocation.tool,
    status: invocation.status,
    ...(invocation.status === 'done' ? { returned: invocation.returned } : {}),
    ...(invocation.error ? { error: invocation.error } : {}),
  });
}

export function cancelSiteToolCall(input: unknown): ActionResult {
  const parsed = cancelSiteTool.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const invocation = invocations.get(parsed.data.invocationId);
  if (!invocation) return failure('SITE_TOOL_INVOCATION_GONE', `No call ${parsed.data.invocationId}: ${GONE}.`);
  if (invocation.status === 'running') invocation.status = 'cancelled';
  return success({ cancelled: invocation.status === 'cancelled' });
}
