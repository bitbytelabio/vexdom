/**
 * `page.harStart` / `page.harStop`: a tab's traffic as HAR 1.2, ported from agent-browser
 * (`native/actions.rs` `har_entry_to_json`, `har_fill_response`, `har_compute_timings`,
 * `har_mime_is_text`, and its `Network.*` event handling). Bodies are capped at 2 MiB each and
 * 32 MiB in all (agent-browser's 64 MiB halved to stay under the Bridge's artifact cap).
 */
import { z } from 'zod';
import { harStart } from '@/lib/actions/page/har-start';
import { harStop } from '@/lib/actions/page/har-stop';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, onDebuggerEvent, onLost, release, send } from './cdp';
import { runningVersion } from './extension-update';

const OWNER = 'har';
const FIREFOX_HINT = 'Recording a HAR needs Chrome’s debugger, which Firefox does not offer to extensions.';

const MAX_ENTRIES = 5000;
const MAX_BODY_BYTES = 2 * 1024 * 1024;
const MAX_TOTAL_BODY_BYTES = 32 * 1024 * 1024;
const MAX_URL_CHARS = 8 * 1024;
const MAX_LABEL_CHARS = 256;
const MAX_HEADER_CHARS = 16 * 1024;

type Content = 'all' | 'text' | 'none';
type Pair = [string, string];

interface HarEntry {
  requestId: string;
  wallTime: number;
  method: string;
  url: string;
  requestHeaders: Pair[];
  postData?: string;
  requestBodySize: number;
  resourceType: string;
  status?: number;
  statusText: string;
  httpVersion: string;
  responseHeaders: Pair[];
  mimeType: string;
  redirectUrl: string;
  responseBodySize: number;
  cdpTiming?: Record<string, number>;
  loadingFinished?: number;
  responseBody?: string;
  responseBodyBase64: boolean;
}

interface Recording {
  content: Content;
  entries: HarEntry[];
  bodyBytes: number;
}

const recordings = new Map<number, Recording>();

const clip = (text: string, max: number) => (text.length > max ? text.slice(0, max) : text);

function headerPairs(headers: unknown): Pair[] {
  const pairs: Pair[] = [];
  let left = MAX_HEADER_CHARS;
  for (const [name, value] of Object.entries((headers ?? {}) as Record<string, unknown>)) {
    if (left <= 0) break;
    const pair: Pair = [clip(name, MAX_LABEL_CHARS), clip(String(value), left)];
    left -= pair[0].length + pair[1].length;
    pairs.push(pair);
  }
  return pairs;
}

export function harMimeIsText(mime: string): boolean {
  const base = (mime.split(';')[0] ?? '').trim().toLowerCase();
  return (
    base.startsWith('text/') ||
    base.endsWith('+json') ||
    base.endsWith('+xml') ||
    [
      'application/json',
      'application/xml',
      'application/javascript',
      'application/x-javascript',
      'application/ecmascript',
      'application/x-www-form-urlencoded',
      'application/graphql',
    ].includes(base)
  );
}

function httpVersionOf(protocol: string | undefined): string {
  switch ((protocol ?? '').toLowerCase()) {
    case 'h2':
      return 'HTTP/2.0';
    case 'h3':
      return 'HTTP/3.0';
    case 'http/1.0':
      return 'HTTP/1.0';
    default:
      return 'HTTP/1.1';
  }
}

function fillResponse(entry: HarEntry, response: Record<string, unknown>): void {
  entry.status = typeof response.status === 'number' ? response.status : undefined;
  entry.statusText = clip(String(response.statusText ?? ''), MAX_LABEL_CHARS);
  entry.mimeType = clip(String(response.mimeType ?? ''), MAX_LABEL_CHARS);
  entry.httpVersion = httpVersionOf(response.protocol as string | undefined);
  entry.responseHeaders = headerPairs(response.headers);
  entry.redirectUrl = clip(entry.responseHeaders.find(([name]) => name.toLowerCase() === 'location')?.[1] ?? '', MAX_URL_CHARS);
  entry.responseBodySize = typeof response.encodedDataLength === 'number' ? response.encodedDataLength : -1;
  entry.cdpTiming = response.timing as Record<string, number> | undefined;
}

export function harTimings(timing: Record<string, number> | undefined, loadingFinished?: number): [Record<string, number>, number] {
  if (!timing) return [{ send: 0, wait: 0, receive: 0 }, 0];
  const get = (key: string) => (typeof timing[key] === 'number' ? timing[key] : -1);
  const requestTime = get('requestTime');
  const span = (start: number, end: number) => (start >= 0 && end >= 0 ? end - start : -1);
  const dns = span(get('dnsStart'), get('dnsEnd'));
  const connect = span(get('connectStart'), get('connectEnd'));
  const ssl = span(get('sslStart'), get('sslEnd'));
  const sendStart = get('sendStart');
  const sendEnd = get('sendEnd');
  const send = Math.max(sendEnd - sendStart, 0);
  const headersStart = get('receiveHeadersStart');
  const headersEnd = get('receiveHeadersEnd');
  const waitEnd = headersStart >= 0 ? headersStart : headersEnd;
  const wait = sendEnd >= 0 && waitEnd >= sendEnd ? waitEnd - sendEnd : 0;
  const receive =
    loadingFinished !== undefined && requestTime >= 0 && headersEnd >= 0
      ? Math.max((loadingFinished - (requestTime + headersEnd / 1000)) * 1000, 0)
      : 0;
  const blocked =
    get('dnsStart') > 0 ? get('dnsStart') : get('connectStart') > 0 ? get('connectStart') : sendStart > 0 ? sendStart : -1;
  const total = [blocked > 0 ? blocked : 0, Math.max(dns, 0), Math.max(connect, 0), send, wait, receive].reduce((a, b) => a + b, 0);
  const timings: Record<string, number> = { send, wait, receive };
  if (blocked > 0) timings.blocked = blocked;
  if (dns >= 0) timings.dns = dns;
  if (connect >= 0) timings.connect = connect;
  if (ssl >= 0) timings.ssl = ssl;
  return [timings, total];
}

function entryJson(entry: HarEntry): Record<string, unknown> {
  const named = (pairs: Pair[]) => pairs.map(([name, value]) => ({ name, value }));
  const cookieHeader = entry.requestHeaders.find(([name]) => name.toLowerCase() === 'cookie')?.[1];
  const requestCookies = (cookieHeader ?? '')
    .split(';')
    .map((pair) => pair.trim())
    .filter(Boolean)
    .map((pair) => {
      const at = pair.indexOf('=');
      return at < 0 ? { name: pair, value: '' } : { name: pair.slice(0, at).trim(), value: pair.slice(at + 1).trim() };
    });
  let queryString: { name: string; value: string }[] = [];
  try {
    queryString = [...new URL(entry.url).searchParams].map(([name, value]) => ({ name, value }));
  } catch {
    queryString = [];
  }
  const responseCookies = entry.responseHeaders
    .filter(([name]) => name.toLowerCase() === 'set-cookie')
    .map(([, value]) => {
      const nameValue = value.split(';')[0] ?? '';
      const at = nameValue.indexOf('=');
      return at < 0
        ? { name: nameValue.trim(), value: '' }
        : { name: nameValue.slice(0, at).trim(), value: nameValue.slice(at + 1).trim() };
    });
  const [timings, time] = harTimings(entry.cdpTiming, entry.loadingFinished);
  const request: Record<string, unknown> = {
    method: entry.method,
    url: entry.url,
    httpVersion: entry.httpVersion,
    cookies: requestCookies,
    headers: named(entry.requestHeaders),
    queryString,
    headersSize: -1,
    bodySize: entry.requestBodySize,
  };
  if (entry.postData !== undefined) {
    const mimeType = entry.requestHeaders.find(([name]) => name.toLowerCase() === 'content-type')?.[1] ?? 'text/plain';
    request.postData = { mimeType, text: entry.postData };
  }
  const content: Record<string, unknown> = {
    size: entry.responseBodySize,
    mimeType: entry.mimeType || 'application/octet-stream',
  };
  if (entry.responseBody !== undefined) {
    content.text = entry.responseBody;
    if (entry.responseBodyBase64) content.encoding = 'base64';
  }
  return {
    startedDateTime: new Date(entry.wallTime > 0 ? entry.wallTime * 1000 : Date.now()).toISOString(),
    time,
    request,
    response: {
      status: entry.status ?? 0,
      statusText: entry.statusText,
      httpVersion: entry.httpVersion,
      cookies: responseCookies,
      headers: named(entry.responseHeaders),
      content,
      redirectURL: entry.redirectUrl,
      headersSize: -1,
      bodySize: entry.responseBodySize,
    },
    cache: {},
    timings,
    _resourceType: entry.resourceType,
  };
}

function browserOf(): { name: string; version: string } {
  const agent = globalThis.navigator?.userAgent ?? '';
  const edge = /Edg\/([\d.]+)/.exec(agent);
  if (edge) return { name: 'Microsoft Edge', version: edge[1] };
  const chrome = /Chrome\/([\d.]+)/.exec(agent);
  return chrome ? { name: 'Chrome', version: chrome[1] } : { name: 'Chromium', version: '' };
}

export async function startHar(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = harStart.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  if (recordings.has(tabId)) return failure('HAR_ACTIVE', 'A HAR recording is already running on this tab; stop it first.');
  const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  try {
    await send({ tabId }, 'Network.enable', { maxTotalBufferSize: 100_000_000, maxResourceBufferSize: 10_000_000 });
  } catch (error) {
    await release(tabId, OWNER);
    return failure('DEBUGGER_UNAVAILABLE', error instanceof Error ? error.message : String(error));
  }
  recordings.set(tabId, { content: parsed.data.content, entries: [], bodyBytes: 0 });
  return success({ started: true });
}

/** The HAR document, ended: `{log: {...}}`. */
export async function stopHar(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = harStop.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const recording = recordings.get(tabId);
  if (!recording) {
    return failure('NO_HAR_RECORDING', 'No HAR recording is active on this tab; start one with page.harStart first.');
  }
  recordings.delete(tabId);
  await release(tabId, OWNER);
  const har = {
    log: {
      version: '1.2',
      creator: { name: 'vexdom', version: runningVersion() },
      browser: browserOf(),
      entries: recording.entries.map(entryJson),
    },
  };
  const requestCount = recording.entries.length;
  if (parsed.data.inline) return success({ requestCount, har });
  return success({ requestCount, artifact: har, file: { stem: 'har', ext: 'har' } });
}

function entryFor(tabId: number, requestId: string): HarEntry | undefined {
  const entries = recordings.get(tabId)?.entries ?? [];
  for (let at = entries.length - 1; at >= 0; at--) if (entries[at].requestId === requestId) return entries[at];
  return undefined;
}

async function keepBody(tabId: number, entry: HarEntry): Promise<void> {
  const recording = recordings.get(tabId);
  if (!recording || recording.content === 'none') return;
  if (recording.content === 'text' && !harMimeIsText(entry.mimeType)) return;
  try {
    const held = await send<{ body: string; base64Encoded: boolean }>({ tabId }, 'Network.getResponseBody', {
      requestId: entry.requestId,
    });
    if (held.body.length > MAX_BODY_BYTES || recording.bodyBytes + held.body.length > MAX_TOTAL_BODY_BYTES) return;
    recording.bodyBytes += held.body.length;
    entry.responseBody = held.body;
    entry.responseBodyBase64 = held.base64Encoded;
  } catch {
    // Chrome has already let the body go.
  }
}

function receive(tabId: number, method: string, params: Record<string, unknown>): void {
  const recording = recordings.get(tabId);
  if (!recording) return;
  const requestId = String(params.requestId ?? '');
  switch (method) {
    case 'Network.requestWillBeSent': {
      const request = (params.request ?? {}) as Record<string, unknown>;
      const redirect = params.redirectResponse as Record<string, unknown> | undefined;
      if (redirect) {
        const previous = entryFor(tabId, requestId);
        if (previous) {
          fillResponse(previous, redirect);
          previous.loadingFinished = typeof params.timestamp === 'number' ? params.timestamp : 0;
        }
      }
      if (recording.entries.length >= MAX_ENTRIES) return;
      let postData = typeof request.postData === 'string' ? request.postData : undefined;
      if (postData !== undefined) {
        if (!postData || postData.length > MAX_BODY_BYTES || recording.bodyBytes + postData.length > MAX_TOTAL_BODY_BYTES) {
          postData = undefined;
        } else {
          recording.bodyBytes += postData.length;
        }
      }
      recording.entries.push({
        requestId,
        wallTime: typeof params.wallTime === 'number' ? params.wallTime : 0,
        method: clip(String(request.method ?? 'GET'), MAX_LABEL_CHARS),
        url: clip(String(request.url ?? ''), MAX_URL_CHARS),
        requestHeaders: headerPairs(request.headers),
        postData,
        requestBodySize: typeof request.postData === 'string' ? request.postData.length : 0,
        resourceType: clip(String(params.type ?? 'Other'), MAX_LABEL_CHARS),
        statusText: '',
        httpVersion: 'HTTP/1.1',
        responseHeaders: [],
        mimeType: '',
        redirectUrl: '',
        responseBodySize: -1,
        responseBodyBase64: false,
      });
      return;
    }
    case 'Network.responseReceived': {
      const entry = entryFor(tabId, requestId);
      if (entry && params.response) fillResponse(entry, params.response as Record<string, unknown>);
      return;
    }
    case 'Network.loadingFinished': {
      const entry = entryFor(tabId, requestId);
      if (!entry) return;
      if (typeof params.timestamp === 'number') entry.loadingFinished = params.timestamp;
      if (typeof params.encodedDataLength === 'number') entry.responseBodySize = params.encodedDataLength;
      void keepBody(tabId, entry);
      return;
    }
    case 'Network.loadingFailed': {
      const entry = entryFor(tabId, requestId);
      if (!entry) return;
      if (entry.status === undefined) {
        entry.status = 0;
        entry.statusText = String(params.errorText ?? 'Failed');
      }
      if (typeof params.timestamp === 'number') entry.loadingFinished = params.timestamp;
    }
  }
}

export function serveHar(): void {
  if (import.meta.env.FIREFOX) return;
  onDebuggerEvent((source, method, params) => {
    if (source.sessionId || source.tabId == null || !method.startsWith('Network.')) return;
    receive(source.tabId, method, (params ?? {}) as Record<string, unknown>);
  });
  onLost((tabId) => recordings.delete(tabId));
}
