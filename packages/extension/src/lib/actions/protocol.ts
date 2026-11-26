import type { AgentKind } from '@/lib/agents/catalog';
import type { FileReport, FileVerdict } from '@/lib/files/report';
import type { MonitorSample } from '@/lib/monitor/events';
import type { RecordedEvent } from '@/lib/recordings/events';
import type { RecordingWorkflow } from '@/lib/recordings/workflow';
import type { PreferenceChange } from '@/lib/settings/preferences';
import type { SkillCategory } from '@/lib/skills/format';
import type { TaskContext } from '@/lib/schedules/task';

export const ACTION_CHANNEL = 'vexdom/action';
export const BRIDGE_CHANNEL = 'vexdom/bridge';

/** 27: the link is Cap'n Proto (crates/vexdom-schema/schema/vexdom.capnp) in binary frames, through ext-core. */
export const SOCKET_PROTOCOL_VERSION = 27;

/** The first protocol whose daemon reads `RunContext.tabs`, the tabs a conversation spans as a turn begins. */
export const TABS_PROTOCOL = 26;

/**
 * The oldest extension the daemon lets in. A store copy updates when its browser decides to, so it
 * and the daemon are routinely a version apart: a change a peer can do without bumps
 * SOCKET_PROTOCOL_VERSION and is sent only to a peer that speaks it, and only a change neither side
 * can do without raises this.
 */
export const MIN_EXTENSION_PROTOCOL = 22;

export const EXTERNAL_RUN_ID = 'external';

export const DAEMON_PORTS = [8785, 8786, 8787] as const;

/** The native messaging host the extension asks to start the daemon when no port answers. */
export const NATIVE_HOST_NAME = 'com.vexdom.bridge';

export interface ActionInvocation {
  channel: typeof ACTION_CHANNEL;
  action: string;
  input?: unknown;
}

export type BridgeRequest =
  | { channel: typeof BRIDGE_CHANNEL; op: 'describe' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'invoke'; action: string; input?: unknown }
  | { channel: typeof BRIDGE_CHANNEL; op: 'pair'; token: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'disconnect' }
  /** A page opened, or someone pressed Connect: pair through the native host. `wait` holds the ask open while a person decides. */
  | { channel: typeof BRIDGE_CHANNEL; op: 'autoPair'; gesture?: boolean; wait?: boolean }
  | { channel: typeof BRIDGE_CHANNEL; op: 'panelOpened' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'saveSkill'; skillId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'removeSkill'; skillId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'nameSession'; sessionId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'recordEvents'; events: RecordedEvent[] }
  | { channel: typeof BRIDGE_CHANNEL; op: 'recordingState' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'analyzeRecording'; recordingId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'monitorSample'; monitorId: string; sample: MonitorSample }
  | { channel: typeof BRIDGE_CHANNEL; op: 'monitorState' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'siteRefusal'; url: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'listSkills'; refresh?: boolean }
  | { channel: typeof BRIDGE_CHANNEL; op: 'agentState'; refresh?: boolean }
  | { channel: typeof BRIDGE_CHANNEL; op: 'setAgent'; agent: AgentKind }
  | { channel: typeof BRIDGE_CHANNEL; op: 'setAgentModel'; agent: AgentKind; model: string | null }
  | { channel: typeof BRIDGE_CHANNEL; op: 'grantAgent'; agent: AgentKind }
  | { channel: typeof BRIDGE_CHANNEL; op: 'preferences' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'setPreference'; change: PreferenceChange }
  | { channel: typeof BRIDGE_CHANNEL; op: 'tasks' }
  | { channel: typeof BRIDGE_CHANNEL; op: 'saveTask'; task: unknown }
  | { channel: typeof BRIDGE_CHANNEL; op: 'deleteTask'; taskId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'runTaskNow'; taskId: string }
  | { channel: typeof BRIDGE_CHANNEL; op: 'pauseTasks'; paused: boolean };

export type ActionResult<T = unknown> =
  | { ok: true; data: T }
  | { ok: false; error: { code: string; message: string } };

export type RunEvent =
  | { kind: 'started'; skill: string; attached?: string; overlays?: string[] }
  | { kind: 'text'; delta: string }
  | { kind: 'tool'; toolId: string; action: string; input: unknown; source?: 'local' | 'external' }
  | { kind: 'approval'; toolId: string; action: string; input: unknown; site?: string }
  | { kind: 'toolResult'; toolId: string; ok: boolean; summary: string }
  | { kind: 'session'; agent: AgentKind; agentSessionId: string | null }
  | { kind: 'attachments'; files: HandedFile[] }
  | { kind: 'usage'; usage: TokenUsage }
  | { kind: 'done'; stopReason: string }
  | { kind: 'error'; code: string; message: string };

/** Token counts as the agent CLI reported them, normalized by its runner. */
export interface TokenUsage {
  /** Tokens occupying the model's context window after its last call — prompt, cache and reply together. */
  contextTokens: number;
  /** Tokens the model generated over the run that reported this. */
  outputTokens: number;
}

/** Which secret the extension is about to prove it holds. The secret itself never crosses the wire. */
export interface AttachedFile {
  id: string;
  name: string;
  mime: string;
  size: number;
  /** Absent while the file analyst is still reading it. */
  report?: FileReport;
  /** The conversation's agent session already holds this report, so it is not handed over again. */
  delivered: boolean;
}

/** A report handed to the agent with this turn. */
export interface HandedFile {
  id: string;
  name: string;
  verdict: FileVerdict;
  code?: string;
}

/** An element the user pointed at with A-Eye. It rides along with the next instruction and scopes it. */
export interface FocusedElement {
  tag: string;
  role?: string;
  selector: string;
  label?: string;
  content: string;
  truncated: boolean;
  url: string;
  title: string;
  /** Data URL of the element photographed at the instant it was picked. */
  shot?: string;
}

export const focusName = (focus: FocusedElement): string => focus.label?.trim() || focus.role || focus.tag;

/** One of the tabs a conversation spans, as its turn begins. */
export interface ConversationTab {
  tabId: number;
  /** What the page calls itself: written by the page, so flattened, clipped and sealed before it is sent. */
  title: string;
  host?: string;
  /** Where an action that names no tab lands. */
  current?: true;
  /** The user put it in the conversation themselves, so its site is one this run is about. */
  chosen?: true;
}

export interface SavedRecording {
  id: string;
  name: string;
  host: string;
  goal?: string;
  steps?: number;
  capturedValues: boolean;
  durationMs: number;
}

export interface RecordingPayload {
  id: string;
  name: string;
  host: string;
  startUrl: string;
  captureValues: boolean;
  durationMs: number;
  events: RecordedEvent[];
}

export interface RunContext {
  url?: string;
  tabId?: number;
  /** The tab-bound conversation this run belongs to. Scopes one-at-a-time and the held conversation. */
  sessionId?: string;
  files?: AttachedFile[];
  recordings?: SavedRecording[];
  /** The agent that issued `agentSessionId`; a different agent cannot resume it. */
  agent?: AgentKind;
  agentSessionId?: string;
  /** Opaque handle from the skill catalog. The daemon resolves it to a skill it enumerated itself and attaches the content at spawn; a path never crosses the wire. */
  agentSkillId?: string;
  /** The element the user pointed at with A-Eye before sending this message. */
  focus?: FocusedElement;
  /**
   * The composer's “Live tool” switch. Off unless the user turned it on for this
   * message, and nothing else can turn it on — it is what lets the agent write and run
   * its own page code, so it stays a deliberate act rather than a capability that is
   * simply there.
   */
  liveTools?: boolean;
  /** Set when a schedule started this run: nobody is watching it, and it should end on one line saying what it found. */
  task?: TaskContext;
  /** The conversation's tabs in tab-strip order, sent only when it spans more than one. */
  tabs?: ConversationTab[];
}

/** One skill from the active agent CLI's own library. Title and handle only — the file's path and content stay on the daemon's side. */
export interface AgentSkillMeta {
  id: string;
  name: string;
  description: string;
}

/** A Vexdom skill projected for the picker: everything but the body. */
export interface CatalogSkill {
  name: string;
  description: string;
  category: SkillCategory;
  domains: string[];
  source: 'bundled' | 'user' | 'uploaded';
}

export interface SkillCatalog {
  /** The agent whose library `agentSkills` came from; switching agents replaces the catalog. */
  agent: AgentKind;
  skills: CatalogSkill[];
  agentSkills: AgentSkillMeta[];
}

export interface RecordingAnalysis {
  workflow: RecordingWorkflow;
  warnings: string[];
}

export interface SavedSkill {
  name: string;
  path?: string;
  replaced?: boolean;
}

const INSTALL_ID = /^[A-Za-z0-9_-]{16,64}$/;

export function isInstallId(value: unknown): value is string {
  return typeof value === 'string' && INSTALL_ID.test(value);
}

export const success = <T>(data: T): ActionResult<T> => ({ ok: true, data });

export const failure = (code: string, message: string): ActionResult<never> => ({
  ok: false,
  error: { code, message },
});

export function isActionInvocation(message: unknown): message is ActionInvocation {
  return isOnChannel(message, ACTION_CHANNEL) && typeof (message as ActionInvocation).action === 'string';
}

export function isBridgeRequest(message: unknown): message is BridgeRequest {
  return isOnChannel(message, BRIDGE_CHANNEL);
}

function isOnChannel(message: unknown, channel: string): boolean {
  return (
    typeof message === 'object' &&
    message !== null &&
    (message as { channel?: unknown }).channel === channel
  );
}
