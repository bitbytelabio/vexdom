/**
 * The shape of the guardrail settings screen, shared by the side panel that renders it
 * and the daemon that fills it in.
 *
 * The screen is a list of overrides, not a list of switches that turn protection on. A
 * row with no `override` is a row using whatever Vexdom ships, which is what a fresh
 * install has everywhere — so the settings tab starts entirely untouched and nothing
 * about the default posture depends on someone having visited it.
 *
 * `locked` rows are shown and explained but cannot be changed from the panel. They are
 * not preferences: allowing `javascript:` navigations or a credential in a query string
 * has no use that is worth a switch someone can hit by accident. Editing
 * `~/.vexdom/config.json` by hand still works, for whoever genuinely means it.
 */

export type RuleEffect = 'allow' | 'confirm' | 'deny';

/** A standing "Always on this site" approval: `action` is a page action, or `*` for everything there. */
export interface ApprovalGrant {
  action: string;
  host: string;
  /** ISO time it was granted. */
  at: string;
}

export interface GuardrailToggle {
  /** Rule id, as it appears in `guardrails.rules` in config.json. */
  readonly id: string;
  readonly title: string;
  /** What the agent is told when this fires. Doubles as the row's explanation. */
  readonly reason: string;
  /** The effect with nothing overridden — what the row falls back to when switched off. */
  readonly fallback: RuleEffect;
  /** Present only when the user has overridden it. Absent is the default state. */
  readonly override?: RuleEffect;
  /** Shown, explained, not changeable from the panel. */
  readonly locked?: boolean;
}

export interface GuardrailSwitch {
  readonly enabled: boolean;
  readonly overridden: boolean;
}

export interface GuardrailSettings {
  readonly rules: readonly GuardrailToggle[];
  /** Marking page text as untrusted data on its way to the model. */
  readonly fence: GuardrailSwitch;
  /** What a `confirm` becomes for a caller with nobody to ask. */
  readonly unattended: { readonly effect: 'allow' | 'deny'; readonly overridden: boolean };
  /** Standing host allowlist added to every run's scope. Edited in config.json. */
  readonly hosts: readonly string[];
  /** Where an override is written, so the panel can say so. */
  readonly configPath: string;
  /** How a side-panel run asks before acting. Absent from a Bridge before permission modes. */
  readonly mode?: PermissionMode;
}

export type PermissionMode = 'rules' | 'ask' | 'plan' | 'auto';

export const PERMISSION_MODES: readonly PermissionMode[] = ['ask', 'plan', 'rules', 'auto'];

export const isPermissionMode = (value: unknown): value is PermissionMode =>
  value === 'rules' || value === 'ask' || value === 'plan' || value === 'auto';

export const MODE_LABEL: Record<PermissionMode, { title: string; note: string }> = {
  ask: { title: 'Ask before acting', note: 'Asks the first time it acts on each site in this conversation.' },
  plan: {
    title: 'Follow a plan',
    note: 'Shows its plan and the sites it needs first, then asks before going anywhere else.',
  },
  rules: { title: 'Use my rules', note: 'Acts freely and asks only where your Guardrails rules say Ask.' },
  auto: { title: 'Act without asking', note: 'Never stops to ask. What your rules block stays blocked.' },
};

/**
 * What a row writes. A rule takes an effect, the two switches take a boolean or a
 * side, and `null` clears the override and puts the row back on the shipped default.
 */
export type GuardrailValue = RuleEffect | boolean | PermissionMode | null;

export const FENCE_SETTING = 'fence';
export const UNATTENDED_SETTING = 'unattended';
/** The side panel's permission mode; a run takes it when it starts. */
export const MODE_SETTING = 'mode';

/** Settings that are not a rule id. Written to their own config keys. */
export const SWITCH_SETTINGS: readonly string[] = [FENCE_SETTING, UNATTENDED_SETTING];

export function isRuleEffect(value: unknown): value is RuleEffect {
  return value === 'allow' || value === 'confirm' || value === 'deny';
}

export const EFFECT_LABEL: Record<RuleEffect, string> = {
  allow: 'Allow',
  confirm: 'Ask',
  deny: 'Block',
};
