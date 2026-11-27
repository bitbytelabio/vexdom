import { browser } from 'wxt/browser';
import { hashManifest } from '@/lib/actions/manifest';
import { describeOwnActions } from '@/lib/bridge/own-actions';
import {
  clientProof,
  isNonce,
  newNonce,
  openSessionKey,
  pairingSecret,
  sameProof,
  serverProof,
  type Transcript,
} from '@/lib/actions/handshake';
import type { ApprovalGrant } from '@/lib/settings/guardrails';
import {
  APPROVALS_PROTOCOL,
  DAEMON_PORTS,
  SOCKET_PROTOCOL_VERSION,
  failure,
  success,
  type ActionResult,
  type RecordingAnalysis,
  type RecordingPayload,
  type RunContext,
  type RunEvent,
  type SavedSkill,
  type SkillCatalog,
} from '@/lib/actions/protocol';
import type { AgentKind, AgentState } from '@/lib/agents/catalog';
import type { FileReport } from '@/lib/files/report';
import type { PreferenceChange, Preferences } from '@/lib/settings/preferences';
import type { SkillDraft } from '@/lib/skills/format';
import type { SiteMapDraft } from '@/lib/skills/site-map';
import type { TaskList, TaskOrder, TaskResult } from '@/lib/schedules/task';
import type { Approver, PairingMode, RefusalCode } from '@/lib/pairing';
import {
  linkClose,
  linkEvents,
  linkFeed,
  linkHello,
  linkOpen,
  linkProve,
  linkPump,
  linkReply,
  linkReplyArtifact,
  linkReplyScreenshot,
  linkRequest,
  linkTrust,
  loadCore,
  type ExtensionRequest,
  type LinkEvent,
} from './core';
import { NOTICE_PROTOCOL, type DeviceNotice } from '@/lib/notices';
import { identify } from './identity';
import type { BridgeReach } from './native-pair';
import { invokeForHarness } from './invoke';
import { wakeDaemon } from './native-wake';

export const DAEMON_STATE_KEY = 'vexdom/daemon';
export const RECONNECT_ALARM = 'vexdom/reconnect';
export const TASKS_KEY = 'vexdom/tasks';
const SESSION_KEY_STORE = 'vexdom/sessionKey';
/** Set when the Bridge said it holds no session for the stored key: kept until a new one replaces it, but no longer worth dialling with. */
const STALE_KEY_STORE = 'vexdom/staleKey';

const BASE_DELAY_MS = 1_000;
const MAX_DELAY_MS = 30_000;
const STABLE_AFTER_MS = 30_000;
const HANDSHAKE_TIMEOUT_MS = 5_000;
const PAIRING_TIMEOUT_MS = 20_000;
const WOKEN_RETRY_MS = 500;

/** The secret itself stays here; only a proof derived from it ever reaches the socket. */
interface Credential {
  kind: 'pair' | 'session';
  secret: string;
}

interface Refusal {
  reason: string;
  retryable: boolean;
  code?: RefusalCode;
}

interface Attempt {
  credential: Credential;
  port: number;
  hello: { extensionVersion: string; manifestHash: string; nonce: string };
  serverNonce?: string;
  secret?: string;
  done(): void;
  /** Abandon this port and try the next one, because the peer never proved it is the daemon. */
  walk(refusal?: Refusal): void;
}

export interface DaemonState {
  connected: boolean;
  paired: boolean;
  port?: number;
  daemonVersion?: string;
  /** What the Bridge speaks. Absent from a Bridge before 0.8, which speaks 22 or less. */
  protocolVersion?: number;
  manifestInSync?: boolean;
  /** Which agent CLI the daemon runs, and what each one's state is. Pushed on connect. */
  agent?: AgentState;
  /** What the / picker offers: the daemon's skills plus the active agent CLI's own. Pushed on connect and when either changes. */
  skillCatalog?: SkillCatalog;
  /** The settings config.json holds for every surface. Pushed on connect and whenever the file changes. */
  preferences?: Preferences;
  error?: string;
  /** Whether this browser can reach Vexdom Bridge's native host. Absent until it has been asked. */
  bridge?: BridgeReach;
  /** Where pairing by itself stands, while this browser is not paired. */
  pairing?: PairingState;
  lastChangeAt: number;
}

export interface PairingState {
  mode?: PairingMode;
  /** Waiting for someone to allow this browser, in the app or in a terminal. */
  wait?: { approver: Approver };
  /** This computer unpaired or turned down this browser; only Reconnect or a code brings it back. */
  blocked?: 'revoked' | 'denied';
  /** The user pressed Disconnect here, so it does not pair again by itself. */
  manual?: boolean;
  error?: string;
}

let socket: WebSocket | null = null;
let welcomedSocket: WebSocket | null = null;
let daemonProtocol = 0;
let attempts = 0;
let connecting: Promise<void> | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
let stableTimer: ReturnType<typeof setTimeout> | undefined;
let runListener: ((runId: string, event: RunEvent) => void) | null = null;
const welcomeListeners = new Set<(daemonVersion: string) => void>();
let draftListener: ((runId: string, draft: SiteMapDraft) => void) | null = null;
let taskRunner: ((order: TaskOrder) => Promise<ActionResult>) | null = null;
let preferencesListener: ((preferences: Preferences) => void) | null = null;
const staleKeyListeners = new Set<() => void>();

const closedListeners = new Set<() => void>();
const openNoticeListeners = new Set<(noticeId: string) => void>();
/** The Bridge answers at once; past this the browser shows its own card instead. */
const NOTICE_TIMEOUT_MS = 3_000;

const listen = <T>(listeners: Set<T>, listener: T): (() => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** The link to a Bridge that had welcomed us went down. */
export const onDaemonClosed = (listener: () => void) => listen(closedListeners, listener);

export function onPreferences(listener: (preferences: Preferences) => void): void {
  preferencesListener = listener;
}

export function onTaskOrder(runner: (order: TaskOrder) => Promise<ActionResult>): void {
  taskRunner = runner;
}

export function onSiteMapDraft(listener: (runId: string, draft: SiteMapDraft) => void): void {
  draftListener = listener;
}

export function onRunEvent(listener: (runId: string, event: RunEvent) => void): void {
  runListener = listener;
}

/** The Bridge proved itself and does not know the stored key: a wiped or reinstalled Bridge, to pair with again. */
export const onStaleKey = (listener: () => void) => listen(staleKeyListeners, listener);

export async function sessionKeyUsable(): Promise<boolean> {
  const stored = await browser.storage.session.get(STALE_KEY_STORE);
  return !stored[STALE_KEY_STORE] && !!(await storedSessionKey());
}

export function onWelcome(listener: (daemonVersion: string) => void): void {
  welcomeListeners.add(listener);
}

const ANALYZE_TIMEOUT_MS = 90_000;
const RECORDING_TIMEOUT_MS = 120_000;
const TITLE_TIMEOUT_MS = 20_000;
const SKILL_TIMEOUT_MS = 15_000;
const AGENT_TIMEOUT_MS = 20_000;
/** The Bridge drops a link silent for 60 s; a ping every 20 s keeps it, and this worker, awake. */
const PING_MS = 20_000;
const OFFLINE = 'No Vexdom daemon is attached — pair the browser first.';
const NO_ANSWER = 'The daemon did not answer in time.';

const pendingReplies = new Map<number, (result: ActionResult<unknown>) => void>();
let pingTimer: ReturnType<typeof setInterval> | undefined;

/** Queues one ExtensionSession call on the welcomed link and sends it; null when there is no such link. */
function post(request: ExtensionRequest): number | null {
  const ws = socket;
  if (!ws || ws !== welcomedSocket || ws.readyState !== WebSocket.OPEN) return null;
  const sent = linkRequest(request);
  flush(ws);
  return sent;
}

/** One ExtensionSession call, resolved with its result, or with a failure once `timeoutMs` passes. */
function call<T>(
  request: ExtensionRequest,
  timeoutMs: number,
  timeoutMessage = NO_ANSWER,
  offline = OFFLINE,
): Promise<ActionResult<T>> {
  const sent = post(request);
  if (sent === null) return Promise.resolve(failure('EXTENSION_OFFLINE', offline));
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      pendingReplies.delete(sent);
      resolve(failure('TIMEOUT', timeoutMessage));
    }, timeoutMs);
    pendingReplies.set(sent, (result) => {
      clearTimeout(timer);
      resolve(result as ActionResult<T>);
    });
  });
}

/** The link went down: nothing in flight will be answered. */
function abandonReplies(): void {
  const waiting = [...pendingReplies.values()];
  pendingReplies.clear();
  for (const settle of waiting) settle(failure('EXTENSION_OFFLINE', OFFLINE));
}

export function sendInstruction(text: string, context?: RunContext): string | null {
  const id = crypto.randomUUID();
  return post({ tag: 'Instruct', id, text, context: context ? JSON.stringify(context) : '' }) === null ? null : id;
}

/** A file the side panel holds, its bytes base64 as the file store keeps them. */
export interface FileToAnalyze {
  fileId: string;
  sessionId: string;
  name: string;
  mime: string;
  size: number;
  content: string;
}

const bytesOf = (base64: string): Uint8Array => Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));

/** One screencast frame of a video recording, for the Bridge's encoder. */
export function sendRecordFrame(recordingId: string, frame: Uint8Array, atMs: number): void {
  post({ tag: 'RecordFrame', recordingId, frame, atMs });
}

export function analyzeFile(file: FileToAnalyze): Promise<ActionResult<FileReport>> {
  return call(
    { tag: 'AnalyzeFile', id: crypto.randomUUID(), ...file, content: bytesOf(file.content) },
    ANALYZE_TIMEOUT_MS,
    'The daemon did not return a report in time.',
  );
}

/** Stops the analyst reading a file that is going away. Nothing to stop when the daemon is gone. */
export function cancelAnalysis(fileId: string): void {
  post({ tag: 'CancelAnalysis', id: crypto.randomUUID(), fileId });
}

export function analyzeRecording(recording: RecordingPayload): Promise<ActionResult<RecordingAnalysis>> {
  return call(
    { tag: 'AnalyzeRecording', recording: JSON.stringify(recording) },
    RECORDING_TIMEOUT_MS,
    'The daemon did not return a workflow in time.',
  );
}

/** Someone clicked a notice the Vexdom app showed for this browser. */
export const onOpenNotice = (listener: (noticeId: string) => void) => listen(openNoticeListeners, listener);

/** True when the Vexdom app took the notice to show as the computer's own; false sends the caller to its own card. */
export async function sendNotice(notice: DeviceNotice): Promise<boolean> {
  if (!daemonSpeaks(NOTICE_PROTOCOL)) return false;
  const result = await call<boolean>({ tag: 'Notice', notice: JSON.stringify(notice) }, NOTICE_TIMEOUT_MS);
  return result.ok && result.data === true;
}

export function nameSession(session: { host?: string; messages: string[] }): Promise<ActionResult<{ title: string }>> {
  return call(
    { tag: 'NameSession', host: session.host ?? '', messages: session.messages },
    TITLE_TIMEOUT_MS,
    'The daemon did not return a name in time.',
  );
}

export function saveSkill(skill: SkillDraft): Promise<ActionResult<SavedSkill>> {
  return call({ tag: 'SaveSkill', skill: JSON.stringify(skill) }, SKILL_TIMEOUT_MS);
}

export function deleteSkill(name: string): Promise<ActionResult<SavedSkill>> {
  return call({ tag: 'DeleteSkill', name }, SKILL_TIMEOUT_MS);
}

export function deleteSiteMap(name: string): Promise<ActionResult<SavedSkill>> {
  return call({ tag: 'DeleteSiteMap', name }, SKILL_TIMEOUT_MS);
}

export function activateSiteMap(stagingId: string, exactHost = false): Promise<ActionResult<SavedSkill>> {
  return call({ tag: 'ActivateSiteMap', stagingId, exactHost }, SKILL_TIMEOUT_MS);
}

export function discardSiteMap(stagingId: string): Promise<ActionResult<SavedSkill>> {
  return call({ tag: 'DiscardSiteMap', stagingId }, SKILL_TIMEOUT_MS);
}

async function agentOp(request: ExtensionRequest): Promise<ActionResult<AgentState>> {
  const result = await call<AgentState>(request, AGENT_TIMEOUT_MS);
  if (result.ok) await setState({ agent: result.data, lastChangeAt: Date.now() });
  return result;
}

export function readAgentState(refresh = false): Promise<ActionResult<AgentState>> {
  return agentOp({ tag: 'AgentState', refresh });
}

export function chooseAgent(agent: AgentKind): Promise<ActionResult<AgentState>> {
  return agentOp({ tag: 'SetAgent', agent });
}

export function chooseAgentModel(agent: AgentKind, model: string | null): Promise<ActionResult<AgentState>> {
  return agentOp({ tag: 'SetAgentModel', agent, model: model ?? '' });
}

export function setUpAgent(agent: AgentKind): Promise<ActionResult<AgentState>> {
  return agentOp({ tag: 'GrantAgent', agent });
}

export async function readSkillCatalog(refresh = false): Promise<ActionResult<SkillCatalog>> {
  const result = await call<SkillCatalog>({ tag: 'ListSkills', refresh }, AGENT_TIMEOUT_MS);
  if (result.ok) await setState({ skillCatalog: result.data, lastChangeAt: Date.now() });
  return result;
}

async function preferenceOp(request: ExtensionRequest): Promise<ActionResult<Preferences>> {
  const result = await call<Preferences>(request, AGENT_TIMEOUT_MS);
  if (result.ok) {
    await setState({ preferences: result.data, lastChangeAt: Date.now() });
    preferencesListener?.(result.data);
  }
  return result;
}

export function readPreferences(): Promise<ActionResult<Preferences>> {
  return preferenceOp({ tag: 'Preferences' });
}

export function setPreference(change: PreferenceChange): Promise<ActionResult<Preferences>> {
  return preferenceOp({ tag: 'SetPreference', change: JSON.stringify(change) });
}

const APPROVALS_UNSUPPORTED = 'Update Vexdom Bridge to manage approvals here (vexdom update).';

/** The standing "Always on this site" approvals the Bridge keeps. */
export async function readApprovals(): Promise<ActionResult<{ grants: ApprovalGrant[] }>> {
  if (!daemonSpeaks(APPROVALS_PROTOCOL)) return failure('UNSUPPORTED', APPROVALS_UNSUPPORTED);
  return call<{ grants: ApprovalGrant[] }>({ tag: 'Approvals' }, AGENT_TIMEOUT_MS);
}

/** Forgets one site's standing approvals, or every site's. */
export async function forgetApprovals(host?: string): Promise<ActionResult<{ removed: number }>> {
  if (!daemonSpeaks(APPROVALS_PROTOCOL)) return failure('UNSUPPORTED', APPROVALS_UNSUPPORTED);
  return call<{ removed: number }>({ tag: 'ForgetApprovals', host: host ?? '' }, AGENT_TIMEOUT_MS);
}

export type TaskOpInput =
  | { t: 'tasks' }
  | { t: 'saveTask'; task: unknown }
  | { t: 'deleteTask'; taskId: string }
  | { t: 'runTaskNow'; taskId: string }
  | { t: 'pauseTasks'; paused: boolean };

function taskRequest(request: TaskOpInput): ExtensionRequest {
  switch (request.t) {
    case 'tasks':
      return { tag: 'Tasks' };
    case 'saveTask':
      return { tag: 'SaveTask', task: JSON.stringify(request.task) };
    case 'deleteTask':
      return { tag: 'DeleteTask', taskId: request.taskId };
    case 'runTaskNow':
      return { tag: 'RunTaskNow', taskId: request.taskId };
    case 'pauseTasks':
      return { tag: 'PauseTasks', paused: request.paused };
  }
}

export async function taskOp(request: TaskOpInput): Promise<ActionResult<TaskList>> {
  const result = await call<TaskList>(
    taskRequest(request),
    AGENT_TIMEOUT_MS,
    NO_ANSWER,
    'No Vexdom daemon is attached — scheduled tasks live there, so start it or pair the browser.',
  );
  if (result.ok) await browser.storage.local.set({ [TASKS_KEY]: result.data });
  return result;
}

export function reportTaskDone(taskId: string, result: TaskResult): boolean {
  return post({ tag: 'TaskDone', taskId, result: JSON.stringify(result) }) !== null;
}

export function cancelRun(id: string): boolean {
  return post({ tag: 'Cancel', id }) !== null;
}

export function sendDecision(id: string, toolId: string, allow: boolean, remember?: boolean): void {
  post({ tag: 'Decision', id, toolId, allow, remember: remember === true });
}

export function resetConversation(sessionId?: string): void {
  post({ tag: 'Reset', sessionId: sessionId ?? '' });
}

/**
 * The worker's start, the reconnect alarm and a retry routinely ask in the same instant — a woken
 * worker always does — and two dials from one browser make the daemon supersede each in turn.
 */
export function connectDaemon(): Promise<void> {
  return (connecting ??= reconnect().finally(() => (connecting = null)));
}

async function reconnect(): Promise<void> {
  if (isLive()) return;
  const sessionKey = await storedSessionKey();
  if (!sessionKey) {
    await setState({ connected: false, paired: false, error: undefined, lastChangeAt: Date.now() });
    return;
  }
  // A daemon that proved itself and still refused this key will refuse it again. Wait for the user
  // to pair rather than discarding a credential on the word of a peer that never proved anything.
  const state = await readState();
  if ((state && !state.paired && state.error) || isLive()) return;
  clearTimeout(reconnectTimer);
  dial({ kind: 'session', secret: sessionKey }, DAEMON_PORTS, 0);
}

export async function pairDaemon(pairingToken: string): Promise<{ ok: boolean; error?: string }> {
  const code = pairingToken.replace(/[\s-]/g, '').toUpperCase();
  if (!code) return { ok: false, error: 'Enter the pairing code from "vexdom pair".' };

  disconnectSocket();
  clearTimeout(reconnectTimer);
  attempts = 0;
  return pairWith(code, DAEMON_PORTS);
}

/**
 * Redeems a grant the native host handed over: never typed, so never normalised, and tried first on the
 * port of the Bridge that issued it, which may not be the first one answering.
 */
export function redeemGrant(code: string, port: number): Promise<{ ok: boolean; error?: string }> {
  disconnectSocket();
  clearTimeout(reconnectTimer);
  attempts = 0;
  return pairWith(code, [port, ...DAEMON_PORTS.filter((other) => other !== port)]);
}

function pairWith(code: string, ports: readonly number[]): Promise<{ ok: boolean; error?: string }> {
  settlePairing({ ok: false, error: 'Superseded by a newer pairing attempt.' });
  return new Promise((resolve) => {
    pairingResult = resolve;
    dial({ kind: 'pair', secret: code }, ports, 0);
    pairingTimer = setTimeout(
      () => settlePairing({ ok: false, error: 'No Vexdom daemon is running. Start one with "vexdom pair".' }),
      PAIRING_TIMEOUT_MS,
    );
  });
}

export async function disconnectDaemon(): Promise<void> {
  clearTimeout(reconnectTimer);
  disconnectSocket();
  await browser.storage.local.remove(SESSION_KEY_STORE);
  await browser.storage.session.remove(STALE_KEY_STORE);
  await setState({ connected: false, paired: false, error: undefined, agent: undefined, lastChangeAt: Date.now() });
}

/** An update may be what a refusal was waiting for, so the link is tried again with the key it has. */
export async function retryAfterUpdate(): Promise<void> {
  const state = await readState();
  if (state && !state.paired && state.error) await setState({ error: undefined, lastChangeAt: Date.now() });
  await connectDaemon();
}

export const pairingNow = async (): Promise<PairingState | undefined> => (await readState())?.pairing;

/** Where pairing by itself stands, for the surfaces to show while this browser is not paired. */
export const reportPairing = (bridge: BridgeReach | undefined, pairing: PairingState | undefined): Promise<void> =>
  setState({ bridge, pairing, lastChangeAt: Date.now() });

let pairingResult: ((result: { ok: boolean; error?: string }) => void) | null = null;
let pairingTimer: ReturnType<typeof setTimeout> | undefined;

function settlePairing(result: { ok: boolean; error?: string }): void {
  clearTimeout(pairingTimer);
  pairingResult?.(result);
  pairingResult = null;
}

function isLive(): boolean {
  return !!socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING);
}

function disconnectSocket(): void {
  const closing = socket;
  socket = null;
  if (!closing) return;
  closing.onopen = null;
  closing.onmessage = null;
  closing.onclose = null;
  clearTimeout(stableTimer);
  stopPinging();
  abandonReplies();
  try {
    closing.close(1000, 'client disconnecting');
  } catch {}
}

function dial(credential: Credential, ports: readonly number[], portIndex: number, carried?: Refusal): void {
  if (portIndex >= ports.length) return giveUp(credential, carried);

  const port = ports[portIndex];
  const ws = new WebSocket(`ws://127.0.0.1:${port}/extension`);
  ws.binaryType = 'arraybuffer';
  socket = ws;
  let settled = false;
  let welcomed = false;
  let refusal = carried;

  const attempt: Attempt = {
    credential,
    port,
    hello: {
      extensionVersion: browser.runtime.getManifest().version,
      manifestHash: hashManifest(describeOwnActions()),
      nonce: newNonce(),
    },
    done() {
      settled = true;
      welcomed = true;
      clearTimeout(handshakeTimer);
    },
    walk(why) {
      if (settled) return;
      settled = true;
      refusal = why ?? carried;
      clearTimeout(handshakeTimer);
      try {
        ws.close(1000, 'unproven daemon');
      } catch {}
    },
  };

  const handshakeTimer = setTimeout(() => attempt.walk(), HANDSHAKE_TIMEOUT_MS);

  ws.onopen = () => {
    void Promise.all([loadCore(), identify()])
      .then(([, identity]) => {
        if (socket !== ws) return;
        linkOpen();
        linkHello({
          protocolVersion: SOCKET_PROTOCOL_VERSION,
          minDaemonProtocol: SOCKET_PROTOCOL_VERSION,
          extensionVersion: attempt.hello.extensionVersion,
          manifestHash: attempt.hello.manifestHash,
          auth: credential.kind,
          nonce: attempt.hello.nonce,
          installId: identity.installId,
          browser: identity.browser ?? '',
        });
        pump(ws, attempt);
      })
      .catch(() => attempt.walk());
  };

  ws.onmessage = (event) => {
    if (socket !== ws || !(event.data instanceof ArrayBuffer)) return;
    linkFeed(new Uint8Array(event.data));
    pump(ws, attempt);
  };

  ws.onclose = (event) => {
    clearTimeout(handshakeTimer);
    // A socket a newer dial replaced speaks for nothing: its close must not mark the live link down.
    if (socket !== ws) return;
    socket = null;
    clearTimeout(stableTimer);
    stopPinging();
    linkClose();
    linkPump();
    linkEvents();
    abandonReplies();
    // A daemon that refuses the handshake outright — a protocol mismatch above all — says why in
    // the close frame and never sends an `unauthorized`. Without this the walk ends on the generic
    // "no daemon is running", which sends the user hunting for a process that is running fine.
    refusal ??= closeRefusal(event);
    // Nothing on this port ever proved itself — a squatter, a stale daemon, an empty port — so the
    // walk continues instead of pinning the extension to whoever answered first.
    if (!welcomed) return dial(credential, ports, portIndex + 1, refusal);
    void setState({ connected: false, paired: true, error: undefined, lastChangeAt: Date.now() });
    for (const listener of closedListeners) listener();
    // A link opened by a code or a grant was welcomed with a session key, so it comes back on that key like any other.
    scheduleRetry();
  };
}

const OWN_CLOSE = new Set(['unproven daemon', '']);

function closeRefusal(event: CloseEvent): Refusal | undefined {
  const reason = event.reason?.trim();
  if (!reason || OWN_CLOSE.has(reason)) return undefined;
  return /protocol version mismatch/i.test(reason) ? { reason, retryable: false } : { reason, retryable: true };
}

function giveUp(credential: Credential, refusal?: Refusal): void {
  if (refusal && !refusal.retryable) {
    settlePairing({ ok: false, error: refusal.reason });
    void refuse(credential, refusal);
    return;
  }
  settlePairing({ ok: false, error: refusal?.reason ?? 'No Vexdom daemon is running.' });
  if (credential.kind !== 'session') return;
  if (refusal) return scheduleRetry();
  void wakeDaemon().then((woken) => {
    if (!woken) return scheduleRetry();
    clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(() => void connectDaemon(), WOKEN_RETRY_MS);
  });
}

async function refuse(credential: Credential, refusal: Refusal): Promise<void> {
  const reason = await adviceFor(refusal.reason);
  const blocked =
    credential.kind === 'session' && refusal.code === 'revoked' ? { pairing: { blocked: 'revoked' as const } } : {};
  await setState({ connected: false, paired: false, error: reason, ...blocked, lastChangeAt: Date.now() });
  if (credential.kind !== 'session' || (refusal.code !== 'unknown' && refusal.code !== 'revoked')) return;
  // Either way the Bridge holds no session for this key; only a Bridge that forgot it is paired with again unasked.
  await browser.storage.session.set({ [STALE_KEY_STORE]: true });
  if (refusal.code === 'unknown') for (const listener of staleKeyListeners) listener();
}

/** A store copy cannot be rebuilt: it is told the store update is on its way, and the store is asked for it now. */
async function adviceFor(reason: string): Promise<string> {
  if (!/protocol version mismatch/i.test(reason)) return reason;
  const unpacked = (await browser.management.getSelf().catch(() => null))?.installType === 'development';
  if (unpacked)
    return `${reason}. Rebuild and reload the extension — “bun run build”, then Reload at chrome://extensions.`;
  if (!import.meta.env.FIREFOX) await browser.runtime.requestUpdateCheck().catch(() => undefined);
  return `${reason}. This copy of Vexdom is older than Vexdom Bridge; the browser installs its update shortly.`;
}

function transcriptOf(attempt: Attempt): Transcript {
  return {
    protocolVersion: SOCKET_PROTOCOL_VERSION,
    extensionVersion: attempt.hello.extensionVersion,
    manifestHash: attempt.hello.manifestHash,
    clientNonce: attempt.hello.nonce,
    serverNonce: attempt.serverNonce ?? '',
  };
}

/** Runs the link until it stalls, sends what it wrote, and acts on what happened. */
function pump(ws: WebSocket, attempt: Attempt): void {
  flush(ws);
  for (const event of linkEvents()) void onEvent(ws, event, attempt);
}

function flush(ws: WebSocket): void {
  const out = linkPump();
  if (out.length && ws.readyState === WebSocket.OPEN) ws.send(out as Uint8Array<ArrayBuffer>);
}

/** A refused hello: a protocol mismatch will be refused again, anything else may pass on another port. */
function refusalOf(reason: string): Refusal {
  return { reason, retryable: !/protocol version mismatch/i.test(reason) };
}

async function onEvent(ws: WebSocket, event: LinkEvent, attempt: Attempt): Promise<void> {
  switch (event.tag) {
    case 'ServerNonce': {
      if (!isNonce(event.nonce) || attempt.serverNonce) return attempt.walk();
      attempt.serverNonce = event.nonce;
      const transcript = transcriptOf(attempt);
      attempt.secret =
        attempt.credential.kind === 'pair'
          ? await pairingSecret(attempt.credential.secret, transcript)
          : attempt.credential.secret;
      if (socket !== ws) return;
      linkProve(await clientProof(attempt.secret, transcript));
      return pump(ws, attempt);
    }

    case 'Refused':
      return attempt.walk(refusalOf(event.reason));

    // Nothing has proved itself yet, so this is a claim, not a verdict: note it and try the next port.
    case 'Unauthorized':
      return attempt.walk({
        reason: event.reason,
        retryable: event.retryable,
        code: event.code ? (event.code as RefusalCode) : undefined,
      });

    case 'Welcome': {
      const secret = attempt.secret;
      if (!secret) return attempt.walk();
      const transcript = transcriptOf(attempt);
      const sealed = event.sealedSessionKey;
      const claims = {
        daemonVersion: event.daemonVersion,
        manifestHash: event.manifestHash,
        manifestInSync: event.manifestInSync,
        sealedSessionKey: sealed,
      };
      if (!sameProof(event.proof, await serverProof(secret, transcript, claims))) {
        return attempt.walk({
          reason: `Whatever is listening on port ${attempt.port} could not prove it is the Vexdom daemon.`,
          retryable: true,
        });
      }
      const sessionKey = sealed ? await openSessionKey(secret, transcript, sealed) : null;
      if (!sessionKey && (sealed || attempt.credential.kind === 'pair')) return attempt.walk();
      if (ws !== socket) return attempt.walk();

      attempt.done();
      welcomedSocket = ws;
      daemonProtocol = event.protocolVersion;
      // Only now may the Bridge's calls reach the page: ext-core holds them until this.
      linkTrust();
      pump(ws, attempt);
      startPinging(ws);
      // A link that is welcomed and then dropped at once has not recovered, and forgetting the
      // backoff on the welcome alone lets two peers take the link from each other every second.
      clearTimeout(stableTimer);
      stableTimer = setTimeout(() => (attempts = 0), STABLE_AFTER_MS);
      if (sessionKey) {
        await browser.storage.local.set({ [SESSION_KEY_STORE]: sessionKey });
        await browser.storage.session.remove(STALE_KEY_STORE);
      }
      settlePairing({ ok: true });
      await setState({
        connected: true,
        paired: true,
        port: attempt.port,
        daemonVersion: event.daemonVersion,
        protocolVersion: event.protocolVersion,
        manifestInSync: event.manifestInSync,
        error: undefined,
        bridge: undefined,
        pairing: undefined,
        lastChangeAt: Date.now(),
      });
      for (const listener of welcomeListeners) listener(event.daemonVersion);
      return;
    }

    case 'Reply': {
      const settle = pendingReplies.get(event.req);
      pendingReplies.delete(event.req);
      settle?.(parseResult(event.result));
      return;
    }

    // ext-core holds both until linkTrust; a link that was never welcomed must not act regardless.
    case 'PageCall':
      if (ws !== welcomedSocket)
        return linkReply(event.callId, JSON.stringify(failure('UNAUTHORIZED', 'The Bridge has not proved itself.')));
      return answerPage(ws, event, attempt);

    case 'Push':
      if (ws !== welcomedSocket) return;
      return onPush(event);

    case 'Closed':
      return;
  }
}

function parseResult(text: string): ActionResult<unknown> {
  try {
    return JSON.parse(text) as ActionResult<unknown>;
  } catch {
    return failure('BRIDGE_ERROR', 'The Bridge answered with something that is not a result.');
  }
}

function parseInput(text: string): unknown {
  if (!text) return undefined;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return undefined;
  }
}

/** A screenshot's image crosses as bytes; its result keeps everything but the data URL. */
function splitImage(result: ActionResult): [string, Uint8Array] {
  if (!result.ok || !result.data || typeof result.data !== 'object') return [JSON.stringify(result), new Uint8Array()];
  const { dataUrl, ...rest } = result.data as { dataUrl?: unknown };
  if (typeof dataUrl !== 'string') return [JSON.stringify(result), new Uint8Array()];
  const base64 = dataUrl.slice(dataUrl.indexOf(',') + 1);
  return [JSON.stringify(success(rest)), bytesOf(base64)];
}

/** The most one artifact may be (the Bridge's cap too). */
const ARTIFACT_MAX_BYTES = 48 * 1024 * 1024;

/**
 * An artifact action's file, split from its result: `dataUrl` for binary (base64, which sealing
 * leaves alone), or `artifact` for a JSON document, sealed like any result before it gets here.
 */
function splitArtifact(result: ActionResult): [string, Uint8Array] {
  if (!result.ok || !result.data || typeof result.data !== 'object') return [JSON.stringify(result), new Uint8Array()];
  const { dataUrl, artifact, ...rest } = result.data as { dataUrl?: unknown; artifact?: unknown };
  let bytes: Uint8Array = new Uint8Array();
  if (typeof dataUrl === 'string') bytes = bytesOf(dataUrl.slice(dataUrl.indexOf(',') + 1));
  else if (artifact !== undefined) bytes = new TextEncoder().encode(JSON.stringify(artifact, null, 2));
  if (bytes.length > ARTIFACT_MAX_BYTES) {
    return [
      JSON.stringify(
        failure(
          'ARTIFACT_TOO_LARGE',
          `The file came to ${bytes.length} bytes; the most the Bridge takes is ${ARTIFACT_MAX_BYTES}.`,
        ),
      ),
      new Uint8Array(),
    ];
  }
  return [JSON.stringify(success(rest)), bytes];
}

async function answerPage(
  ws: WebSocket,
  page: Extract<LinkEvent, { tag: 'PageCall' }>,
  attempt: Attempt,
): Promise<void> {
  const tabId = page.tabId >= 0 ? page.tabId : undefined;
  const input = parseInput(page.input);
  switch (page.kind) {
    case 'describe':
      linkReply(page.callId, JSON.stringify(success(describeOwnActions())));
      break;
    case 'invoke':
      linkReply(
        page.callId,
        JSON.stringify(await invokeForHarness(page.action, input, tabId, page.runId || undefined)),
      );
      break;
    case 'screenshot': {
      const [result, image] = splitImage(await invokeForHarness('page.screenshot', input, tabId));
      linkReplyScreenshot(page.callId, result, image);
      break;
    }
    case 'artifact': {
      const [result, data] = splitArtifact(await invokeForHarness(page.action, input, tabId));
      linkReplyArtifact(page.callId, result, data);
      break;
    }
    case 'runTask': {
      const result = taskRunner
        ? await taskRunner(input as TaskOrder).catch((error: unknown) => failure('BRIDGE_ERROR', String(error)))
        : failure('UNSUPPORTED', 'This browser is not ready to run scheduled tasks yet.');
      linkReply(page.callId, JSON.stringify(result));
      break;
    }
    default:
      linkReply(
        page.callId,
        JSON.stringify(failure('UNKNOWN_ACTION', `The extension does not answer "${page.kind}".`)),
      );
  }
  if (socket === ws) pump(ws, attempt);
}

async function onPush(push: Extract<LinkEvent, { tag: 'Push' }>): Promise<void> {
  switch (push.kind) {
    case 'run':
      return runListener?.(push.id, parseInput(push.payload) as RunEvent);
    case 'siteMapDraft':
      return draftListener?.(push.id, parseInput(push.payload) as SiteMapDraft);
    case 'openNotice':
      for (const listener of openNoticeListeners) listener(push.id);
      return;
    case 'agentInfo': {
      const result = parseResult(push.payload);
      if (result.ok) await setState({ agent: result.data as AgentState, lastChangeAt: Date.now() });
      return;
    }
    case 'skillCatalog': {
      const result = parseResult(push.payload);
      if (result.ok) await setState({ skillCatalog: result.data as SkillCatalog, lastChangeAt: Date.now() });
      return;
    }
    case 'preferencesInfo': {
      const result = parseResult(push.payload);
      if (!result.ok) return;
      await setState({ preferences: result.data as Preferences, lastChangeAt: Date.now() });
      preferencesListener?.(result.data as Preferences);
      return;
    }
    case 'taskList': {
      const result = parseResult(push.payload);
      if (result.ok) await browser.storage.local.set({ [TASKS_KEY]: result.data });
      return;
    }
    default:
      return;
  }
}

function startPinging(ws: WebSocket): void {
  stopPinging();
  pingTimer = setInterval(() => {
    if (socket !== ws) return stopPinging();
    post({ tag: 'Ping' });
  }, PING_MS);
}

function stopPinging(): void {
  clearInterval(pingTimer);
  pingTimer = undefined;
}

/** Held back until the daemon has welcomed us: mid-handshake it would be read as the proof. */
export function reportFocus(): void {
  if (isWelcomed()) post({ tag: 'Focus' });
}

export function daemonSpeaks(version: number): boolean {
  return isWelcomed() && daemonProtocol >= version;
}

function isWelcomed(): boolean {
  return !!socket && socket === welcomedSocket;
}

function scheduleRetry(): void {
  const backoff = Math.min(MAX_DELAY_MS, BASE_DELAY_MS * 2 ** attempts++);
  const delay = backoff * (0.5 + Math.random() / 2);
  clearTimeout(reconnectTimer);
  reconnectTimer = setTimeout(() => void connectDaemon(), delay);
}

async function readState(): Promise<DaemonState | undefined> {
  const stored = await browser.storage.session.get(DAEMON_STATE_KEY);
  return stored[DAEMON_STATE_KEY] as DaemonState | undefined;
}

let stateQueue: Promise<unknown> = Promise.resolve();

/** One writer at a time: the agent and skill pushes right behind a welcome must not write back the `connected` they read before it. */
function setState(patch: Partial<DaemonState> & { lastChangeAt: number }): Promise<void> {
  const write = async () => {
    const current = (await readState()) ?? { connected: false, paired: false, lastChangeAt: 0 };
    await browser.storage.session.set({ [DAEMON_STATE_KEY]: { ...current, ...patch } });
  };
  const run = stateQueue.then(write, write);
  stateQueue = run.catch(() => undefined);
  return run;
}

async function storedSessionKey(): Promise<string | null> {
  const stored = await browser.storage.local.get(SESSION_KEY_STORE);
  const key = stored[SESSION_KEY_STORE];
  return typeof key === 'string' && key ? key : null;
}
