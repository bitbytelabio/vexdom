/**
 * Files made from a tab (ADR-030): `page.pdf`, `page.traceStart`/`page.traceStop` (agent-browser
 * `native/tracing.rs`) and the screencast behind `page.recordStart`/`page.recordStop`, whose
 * frames the Bridge encodes. PDF and trace bytes go back as a base64 `dataUrl`, which the
 * artifact path writes to disk.
 */
import { z } from 'zod';
import { pdf } from '@/lib/actions/page/pdf';
import { recordStart } from '@/lib/actions/page/record-start';
import { traceStart } from '@/lib/actions/page/trace-start';
import { traceStop } from '@/lib/actions/page/trace-stop';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { base64ToBytes, textToBase64 } from './bytes';
import {
  acquire,
  detachedFailure,
  onDebuggerEvent,
  onLost,
  release,
  send,
  withDebugger,
  type DebuggerSession,
} from './cdp';
import { sendRecordFrame } from './socket';
import CURSOR_SCRIPT from './vendor/recording-cursor.js?raw';

const FIREFOX_HINT = 'This needs Chrome’s debugger, which Firefox does not offer to extensions.';
const STOP_DEADLINE_MS = 30_000;
const MAX_PROFILE_EVENTS = 5_000_000;

const DEFAULT_PROFILER_CATEGORIES = [
  'devtools.timeline',
  'disabled-by-default-devtools.timeline',
  'disabled-by-default-devtools.timeline.frame',
  'disabled-by-default-devtools.timeline.stack',
  'v8.execute',
  'disabled-by-default-v8.cpu_profiler',
  'disabled-by-default-v8.cpu_profiler.hires',
  'v8',
  'disabled-by-default-v8.runtime_stats',
  'blink',
  'blink.user_timing',
  'latencyInfo',
  'renderer.scheduler',
  'sequence_manager',
  'toplevel',
];

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function pdfInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = pdf.input.safeParse(input ?? {});
  if (!parsed.success) return Promise.resolve(failure('INVALID_INPUT', z.prettifyError(parsed.error)));
  const { landscape, printBackground, preferCSSPageSize } = parsed.data;
  return withDebugger(tabId, FIREFOX_HINT, async (session) => {
    const printed = await send<{ data: string }>(session, 'Page.printToPDF', {
      landscape,
      printBackground,
      preferCSSPageSize,
    });
    return success({ dataUrl: `data:application/pdf;base64,${printed.data}`, file: { stem: 'page', ext: 'pdf' } });
  });
}

interface Tracing {
  mode: 'trace' | 'profile';
  /** `Profiler.*` instead of `Tracing.*`: the debugger refused tracing. */
  fallback: boolean;
  events: unknown[];
  dropped: boolean;
  complete?: (stream?: string) => void;
}

const traces = new Map<number, Tracing>();
const lostTraces = new Map<number, string>();

export async function startTrace(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = traceStart.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  if (traces.has(tabId))
    return failure('TRACE_ACTIVE', 'A trace or profile is already running on this tab; stop it first.');
  const { mode, categories } = parsed.data;
  const acquired = await acquire(tabId, 'trace', FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  lostTraces.delete(tabId);
  const session = acquired.lease.session;
  const state: Tracing = { mode, fallback: false, events: [], dropped: false };
  try {
    await send(
      session,
      'Tracing.start',
      mode === 'trace'
        ? { traceConfig: { recordMode: 'recordContinuously' }, transferMode: 'ReturnAsStream' }
        : {
            traceConfig: { includedCategories: categories ?? DEFAULT_PROFILER_CATEGORIES, enableSampling: true },
            transferMode: 'ReportEvents',
          },
    );
  } catch (error) {
    if (mode === 'trace') {
      await release(tabId, 'trace');
      return failure('TRACING_UNAVAILABLE', `Chrome’s debugger refused to trace this tab: ${message(error)}`);
    }
    try {
      await send(session, 'Profiler.enable');
      await send(session, 'Profiler.start');
      state.fallback = true;
    } catch (fallbackError) {
      await release(tabId, 'trace');
      return failure(
        'TRACING_UNAVAILABLE',
        `Neither tracing nor the profiler is available here: ${message(fallbackError)}`,
      );
    }
  }
  traces.set(tabId, state);
  return success({ started: true, mode, ...(state.fallback ? { profiler: 'cpu' } : {}) });
}

async function readStream(session: DebuggerSession, handle: string): Promise<string> {
  let text = '';
  for (;;) {
    const chunk = await send<{ data: string; base64Encoded?: boolean; eof: boolean }>(session, 'IO.read', { handle });
    text += chunk.base64Encoded ? new TextDecoder().decode(base64ToBytes(chunk.data)) : chunk.data;
    if (chunk.eof) break;
  }
  await send(session, 'IO.close', { handle }).catch(() => undefined);
  return text;
}

function eventsOf(text: string): unknown[] {
  try {
    const parsed = JSON.parse(text) as { traceEvents?: unknown[] } | unknown[];
    return Array.isArray(parsed) ? parsed : (parsed.traceEvents ?? []);
  } catch {
    return text
      .split('\n')
      .filter(Boolean)
      .flatMap((line) => {
        try {
          const value = JSON.parse(line) as { traceEvents?: unknown[] };
          return value.traceEvents ?? [value];
        } catch {
          return [];
        }
      });
  }
}

export async function stopTrace(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = traceStop.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const state = traces.get(tabId);
  if (!state) {
    const gone = lostTraces.get(tabId);
    lostTraces.delete(tabId);
    return gone
      ? detachedFailure(gone)
      : failure('NO_TRACE', 'No trace or profile is running on this tab; start one with page.traceStart.');
  }
  const session = { tabId };
  try {
    if (state.fallback) {
      const stopped = await send<{ profile: unknown }>(session, 'Profiler.stop');
      await send(session, 'Profiler.disable').catch(() => undefined);
      const json = JSON.stringify(stopped.profile);
      return success({
        mode: state.mode,
        profiler: 'cpu',
        dataUrl: `data:application/json;base64,${textToBase64(json)}`,
        file: { stem: 'profile', ext: 'cpuprofile' },
      });
    }
    const completed = new Promise<string | undefined>((resolve, reject) => {
      state.complete = resolve;
      setTimeout(
        () => reject(new Error(`${state.mode === 'trace' ? 'Tracing' : 'Profiler'} stop timed out after 30s`)),
        STOP_DEADLINE_MS,
      );
    });
    await send(session, 'Tracing.end');
    const stream = await completed;
    if (stream && !state.events.length) state.events.push(...eventsOf(await readStream(session, stream)));
    const json = JSON.stringify({ traceEvents: state.events });
    return success({
      mode: state.mode,
      eventCount: state.events.length,
      ...(state.dropped ? { eventsDropped: true } : {}),
      dataUrl: `data:application/json;base64,${textToBase64(json)}`,
      file: { stem: state.mode, ext: 'json' },
    });
  } catch (error) {
    return failure('ACTION_FAILED', message(error));
  } finally {
    // Kept until here: the events of the stop land on the tab's entry.
    traces.delete(tabId);
    await release(tabId, 'trace');
  }
}

interface Recording {
  recordingId: string;
  cursorScript?: string;
}

const recordings = new Map<number, Recording>();

/** A video of this tab is being recorded, so every action shows its cue whatever the setting says. */
export const isRecordingVideo = (tabId: number): boolean => recordings.has(tabId);

const CURSOR_WORLD = 'vexdom-recording-cursor';

export async function startRecording(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = recordStart.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const { recordingId, cursor, path } = parsed.data;
  if (!recordingId)
    return failure('INVALID_INPUT', 'page.recordStart is started through the Vexdom Bridge, which encodes the video.');
  if (recordings.size) return failure('RECORDING_ACTIVE', 'A recording is already running; stop it first.');
  const acquired = await acquire(tabId, 'record', FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  const session = acquired.lease.session;
  const recording: Recording = { recordingId };
  try {
    await send(session, 'Page.enable');
    if (cursor) {
      const added = await send<{ identifier: string }>(session, 'Page.addScriptToEvaluateOnNewDocument', {
        source: CURSOR_SCRIPT,
        worldName: CURSOR_WORLD,
        runImmediately: true,
      });
      recording.cursorScript = added.identifier;
    }
    await send(session, 'Page.startScreencast', { format: 'jpeg', quality: 80, everyNthFrame: 1 });
  } catch (error) {
    await release(tabId, 'record');
    return failure('ACTION_FAILED', message(error));
  }
  recordings.set(tabId, recording);
  return success({ recording: true, path });
}

export async function stopRecording(tabId: number): Promise<ActionResult> {
  const entry = [...recordings.entries()].find(([held]) => held === tabId) ?? [...recordings.entries()][0];
  if (!entry) return failure('NO_RECORDING', 'No recording is running.');
  const [heldTab, recording] = entry;
  recordings.delete(heldTab);
  const session = { tabId: heldTab };
  await send(session, 'Page.stopScreencast').catch(() => undefined);
  if (recording.cursorScript) {
    await send(session, 'Page.removeScriptToEvaluateOnNewDocument', { identifier: recording.cursorScript }).catch(
      () => undefined,
    );
    await cleanCursor(session);
  }
  await release(heldTab, 'record');
  return success({ stopped: true });
}

async function cleanCursor(session: DebuggerSession): Promise<void> {
  try {
    const tree = await send<{ frameTree: { frame: { id: string } } }>(session, 'Page.getFrameTree');
    const world = await send<{ executionContextId: number }>(session, 'Page.createIsolatedWorld', {
      frameId: tree.frameTree.frame.id,
      worldName: CURSOR_WORLD,
    });
    await send(session, 'Runtime.evaluate', {
      contextId: world.executionContextId,
      expression: 'globalThis.__agentBrowserRecordingCursorCleanup?.()',
    });
  } catch {
    // The page went away with its overlay.
  }
}

export function serveCapture(): void {
  if (import.meta.env.FIREFOX) return;
  onDebuggerEvent((source, method, params) => {
    if (source.sessionId || source.tabId == null) return;
    const tabId = source.tabId;
    if (method === 'Page.screencastFrame') {
      const frame = params as { data: string; sessionId: number };
      const recording = recordings.get(tabId);
      if (recording) sendRecordFrame(recording.recordingId, base64ToBytes(frame.data), Date.now());
      void send(source, 'Page.screencastFrameAck', { sessionId: frame.sessionId }).catch(() => undefined);
      return;
    }
    const trace = traces.get(tabId);
    if (!trace) return;
    if (method === 'Tracing.dataCollected') {
      const value = (params as { value?: unknown[] }).value ?? [];
      if (trace.events.length + value.length > MAX_PROFILE_EVENTS) trace.dropped = true;
      else trace.events.push(...value);
    } else if (method === 'Tracing.tracingComplete') {
      trace.complete?.((params as { stream?: string }).stream);
    }
  });
  onLost((tabId, reason) => {
    if (traces.delete(tabId)) lostTraces.set(tabId, reason);
    recordings.delete(tabId);
  });
}
