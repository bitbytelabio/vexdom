/**
 * `page.routeNetwork` / `page.unrouteNetwork`: answer, block or let through a tab's requests
 * with the Fetch domain (ADR-030). The matching, the mocked response and its CORS handling are
 * agent-browser's (`native/actions.rs` `collapse_wildcards`, `route_url_matches`, `select_route`,
 * `mocked_response`, `build_fetch_patterns`). Routes are per tab, in registration order; the
 * `route` lease is held while a tab has any.
 */
import { z } from 'zod';
import { routeNetwork } from '@/lib/actions/page/route-network';
import { unrouteNetwork } from '@/lib/actions/page/unroute-network';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { textToBase64 } from './bytes';
import { acquire, onDebuggerEvent, onLost, release, send, type DebuggerSession } from './cdp';

const OWNER = 'route';
const FIREFOX_HINT = 'Routing requests needs Chrome’s debugger, which Firefox does not offer to extensions.';

export interface RouteResponse {
  status?: number;
  body?: string;
  contentType?: string;
  headers?: Record<string, string>;
  allowOrigin?: string;
}

export interface Route {
  pattern: string;
  abort: boolean;
  response?: RouteResponse;
  resourceTypes: string[];
}

const routes = new Map<number, Route[]>();

export function collapseWildcards(pattern: string): string {
  return pattern.replace(/\*+/g, '*');
}

export function routeUrlMatches(pattern: string, url: string): boolean {
  if (pattern === '*') return true;
  if (!pattern.includes('*')) return url.includes(pattern);
  const collapsed = collapseWildcards(pattern);
  const parts = collapsed.split('*').filter(Boolean);
  if (!parts.length) return true;
  const anchoredStart = !collapsed.startsWith('*');
  const anchoredEnd = !collapsed.endsWith('*');
  let pos = 0;
  let idx = 0;
  if (anchoredStart) {
    if (!url.startsWith(parts[0])) return false;
    pos = parts[0].length;
    idx = 1;
  }
  for (; idx < parts.length; idx++) {
    const found = url.indexOf(parts[idx], pos);
    if (found < 0) return false;
    pos = found + parts[idx].length;
  }
  return !anchoredEnd || url.endsWith(parts.at(-1)!);
}

/** The first route, in registration order, whose pattern and resource types match and that aborts or answers. */
export function selectRoute(list: Route[], url: string, resourceType: string): Route | undefined {
  return list.find(
    (route) =>
      routeUrlMatches(route.pattern, url) &&
      (!route.resourceTypes.length || route.resourceTypes.some((type) => type.toLowerCase() === resourceType.toLowerCase())) &&
      (route.abort || route.response !== undefined),
  );
}

type Header = { name: string; value: string };

/** Status, headers and base64 body for a mocked response; CORS headers only for the origin the route allows. */
export function mockedResponse(resp: RouteResponse, requestHeaders?: Record<string, string>): [number, Header[], string] {
  const status = resp.status ?? 200;
  const body = resp.body ?? '';
  let encoded = textToBase64(body);
  const headers: Header[] = [];
  const routeSets = (name: string) => Object.keys(resp.headers ?? {}).some((key) => key.toLowerCase() === name.toLowerCase());
  let contentType = resp.contentType;
  if (contentType === undefined && !routeSets('content-type')) {
    try {
      JSON.parse(body);
      contentType = 'application/json';
    } catch {
      contentType = undefined;
    }
  }
  if (contentType !== undefined) headers.push({ name: 'Content-Type', value: contentType });
  for (const [name, value] of Object.entries(resp.headers ?? {})) headers.push({ name, value });

  const header = (name: string) =>
    Object.entries(requestHeaders ?? {}).find(([key]) => key.toLowerCase() === name.toLowerCase())?.[1];
  const hasAcao = routeSets('access-control-allow-origin');
  let origin = header('origin');
  if (origin === undefined) {
    const referer = header('referer');
    try {
      origin = referer ? new URL(referer).origin : undefined;
    } catch {
      origin = undefined;
    }
  }
  if (origin !== undefined && !(resp.allowOrigin !== undefined && resp.allowOrigin.replace(/\/+$/, '') === origin)) {
    origin = undefined;
  }
  const preflight = header('access-control-request-method');
  if (!hasAcao && origin !== undefined) {
    const add = (name: string, value: string) => {
      if (!routeSets(name)) headers.push({ name, value });
    };
    add('Access-Control-Allow-Origin', origin);
    add('Access-Control-Allow-Credentials', 'true');
    if (preflight !== undefined) {
      add('Access-Control-Allow-Methods', preflight);
      const asked = header('access-control-request-headers');
      if (asked !== undefined) add('Access-Control-Allow-Headers', asked);
    }
  }
  if (preflight !== undefined && !hasAcao && origin !== undefined) {
    encoded = '';
    return [204, headers, encoded];
  }
  return [status, headers, encoded];
}

const patternsOf = (list: Route[]) => list.map((route) => ({ urlPattern: collapseWildcards(route.pattern) }));

export async function routeNetworkInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = routeNetwork.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { pattern, abort, response, resourceTypes } = parsed.data;
  if (!abort && response === undefined) {
    return failure('INVALID_INPUT', 'Give "abort": true or a "response" — a route that does neither changes nothing.');
  }
  const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  const list = [...(routes.get(tabId) ?? []), { pattern, abort, response, resourceTypes }];
  try {
    await send({ tabId }, 'Fetch.enable', { patterns: patternsOf(list) });
  } catch (error) {
    if (!routes.get(tabId)?.length) await release(tabId, OWNER);
    return failure('DEBUGGER_UNAVAILABLE', error instanceof Error ? error.message : String(error));
  }
  routes.set(tabId, list);
  return success({ routed: pattern });
}

export async function unrouteNetworkInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = unrouteNetwork.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { pattern } = parsed.data;
  const list = (routes.get(tabId) ?? []).filter((route) => pattern !== undefined && route.pattern !== pattern);
  if (list.length) {
    routes.set(tabId, list);
    await send({ tabId }, 'Fetch.enable', { patterns: patternsOf(list) }).catch(() => undefined);
  } else if (routes.delete(tabId)) {
    await send({ tabId }, 'Fetch.disable').catch(() => undefined);
    await release(tabId, OWNER);
  }
  return success({ unrouted: pattern ?? 'all' });
}

interface Paused {
  requestId: string;
  request?: { url?: string; headers?: Record<string, string> };
  resourceType?: string;
}

async function answer(source: DebuggerSession, paused: Paused): Promise<void> {
  const route = selectRoute(routes.get(source.tabId) ?? [], paused.request?.url ?? '', paused.resourceType ?? '');
  // No route (or the list was lost with a restarted worker): let it through, never leave it hanging.
  if (!route) {
    await send(source, 'Fetch.continueRequest', { requestId: paused.requestId }).catch(() => undefined);
    return;
  }
  if (route.abort) {
    await send(source, 'Fetch.failRequest', { requestId: paused.requestId, errorReason: 'Failed' }).catch(() => undefined);
    return;
  }
  const [status, headers, body] = mockedResponse(route.response!, paused.request?.headers);
  await send(source, 'Fetch.fulfillRequest', {
    requestId: paused.requestId,
    responseCode: status,
    responseHeaders: headers,
    body,
  }).catch(() => undefined);
}

export function serveNetworkRoutes(): void {
  if (import.meta.env.FIREFOX) return;
  onDebuggerEvent((source, method, params) => {
    if (method === 'Fetch.requestPaused' && source.tabId != null) void answer(source, params as Paused);
  });
  onLost((tabId) => routes.delete(tabId));
}
