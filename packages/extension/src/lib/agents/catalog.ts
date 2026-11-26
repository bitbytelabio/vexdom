export const AGENT_KINDS = ['claude'] as const;

export type AgentKind = (typeof AGENT_KINDS)[number];

export const DEFAULT_AGENT: AgentKind = 'claude';

export interface AgentDescriptor {
  kind: AgentKind;
  label: string;
  vendor: string;
  bin: string;
  install: string;
  docs: string;
  /** Model ids the picker offers, strongest first, whenever the CLI cannot list its own. */
  models: string[];
  /** Shipped before a whole conversation was run against the real CLI; the picker and the docs say so. */
  beta?: boolean;
}

export const AGENTS: Record<AgentKind, AgentDescriptor> = {
  claude: {
    kind: 'claude',
    label: 'Claude Code',
    vendor: 'Anthropic',
    bin: 'claude',
    install: 'npm i -g @anthropic-ai/claude-code',
    docs: 'https://claude.com/claude-code',
    // Claude Code resolves each alias to the newest model of that family, so this list does not age.
    models: ['fable', 'opus', 'sonnet', 'haiku'],
  },
};

export const AGENT_LIST: AgentDescriptor[] = AGENT_KINDS.map((kind) => AGENTS[kind]);

export function isAgentKind(value: unknown): value is AgentKind {
  return typeof value === 'string' && (AGENT_KINDS as readonly string[]).includes(value);
}

/** An agent's name, or its kind as sent when it came after this build: a newer Bridge can run one. */
export function agentLabel(kind: AgentKind): string {
  return isAgentKind(kind) ? AGENTS[kind].label : String(kind);
}

export interface AgentProblem {
  code: 'AGENT_MISSING' | 'AGENT_UNUSABLE' | 'AGENT_NEEDS_PERMISSION';
  message: string;
  /** What the user can do about it — a shell command, a URL, or a line of config. */
  fix?: string;
  /** The daemon can repair this itself if the user asks it to. */
  grantable?: boolean;
}

/**
 * Anything a CLI will take as a model, and nothing it could read as a flag: no leading dash, no
 * whitespace, no control or format characters. Cursor's `claude-opus-4-8[effort=high]` passes.
 */
export function isModelId(value: unknown): value is string {
  return typeof value === 'string' && /^[A-Za-z0-9][^\s\p{C}]{0,199}$/u.test(value);
}

export interface ModelList {
  ids: string[];
  /** `cli` when the agent's own CLI listed them, `catalog` when it could not and these are curated. */
  from: 'cli' | 'catalog';
  /** When the CLI's list was read. */
  at?: number;
  /** Why the last attempt to read it failed. */
  error?: string;
}

export interface RunnerStatus {
  kind: AgentKind;
  bin: string;
  ready: boolean;
  version?: string;
  /** The model this runner will be spawned with; unset means the CLI's own default. */
  model?: string;
  models?: ModelList;
  problem?: AgentProblem;
}

export interface AgentState {
  active: AgentKind;
  runners: RunnerStatus[];
}

export function activeRunner(state: AgentState | undefined): RunnerStatus | undefined {
  return state?.runners.find((runner) => runner.kind === state.active);
}
