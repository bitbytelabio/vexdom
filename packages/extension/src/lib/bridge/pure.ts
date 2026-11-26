/**
 * ext-core's pure half (P9, `crates/vexdom-pure`): the handshake crypto, secret detection, sealing and
 * the secret vault, intent routing, the blocked-sites list, schedule rules, profile and recording-workflow validation, one implementation shared with the Bridge. The
 * calls are synchronous but need the module loaded: an async caller awaits `loadCore()` first, and
 * the side panel and the settings page load it before they render.
 */
import { browser } from 'wxt/browser';
import init, { intentActThreshold, intentRoute } from './ext-core/ext_core.js';

export {
  blockedSitesBlockedBy,
  blockedSitesDescribe,
  blockedSitesParse,
  blockedSitesPatternsFor,
  blockedSitesSubject,
  blockedSitesValid,
  handshakeClientProof,
  handshakeOpenSessionKey,
  handshakePairingSecret,
  handshakeSealSessionKey,
  handshakeServerProof,
  profileAsUser,
  profileIsEmpty,
  profileIsUser,
  profileNormalize,
  scheduleDescribeMoment,
  scheduleDescribeRule,
  scheduleHeadlineOf,
  scheduleInsideWindow,
  scheduleNextAfter,
  scheduleNextRunFor,
  scheduleOrderFor,
  scheduleUpcoming,
  scheduleValidateRule,
  scheduleValidateTask,
  sealHandlesIn,
  secretsCaseFlips,
  secretsEntropy,
  secretsFind,
  secretsKindForKey,
  secretsLooksLikeCredential,
  secretsNotAPlaceholder,
  vaultRelease,
  vaultSeal,
  workflowValidate,
  type SealHandle,
  type SecretSpan,
} from './ext-core/ext_core.js';

let loading: Promise<void> | null = null;

/** Loads the module once from `source`; later calls share the first load. */
export function loadCoreFrom(source: () => Promise<BufferSource | Response>): Promise<void> {
  loading ??= source()
    .then(init)
    .catch((error: unknown) => {
      loading = null;
      throw error;
    });
  return loading;
}

/** A module service worker refuses top-level await, so the module is loaded by the first caller. */
export function loadCore(): Promise<void> {
  return loadCoreFrom(() => fetch((browser.runtime.getURL as (path: string) => string)('/ext-core.wasm')));
}

/** The browser's IANA time zone: schedules run on its wall clock. */
export function localTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** A value as JSON text; what JSON has no text for (`undefined`) is `null`. */
export function jsonText(value: unknown): string {
  return JSON.stringify(value) ?? 'null';
}

export type EscalationReason =
  | 'no-match'
  | 'below-threshold'
  | 'question'
  | 'multi-step'
  | 'conditional'
  | 'consequential'
  | 'skill-prefix';

export interface LocalIntent {
  ruleId: string;
  action: string;
  input: unknown;
  label: string;
  score: number;
}

export type Routing =
  | { decision: 'act'; intent: LocalIntent }
  | { decision: 'escalate'; reason: EscalationReason; score: number; ruleId?: string };

/** How the local grammar routes one instruction: one page action when it is unambiguous, else the agent. */
export function routeIntent(input: string): Routing {
  return JSON.parse(intentRoute(input)) as Routing;
}

/** The score at or above which a match acts rather than escalates. */
export function actThreshold(): number {
  return intentActThreshold();
}
