import { jsonText, workflowValidate } from '@/lib/bridge/pure';

export const MAX_STEPS = 80;
export const MAX_VARIABLES = 20;
export const MAX_CAVEATS = 8;
export const MAX_WORKFLOW_BYTES = 12 * 1024;

export const STEP_LIMITS = {
  goal: 160,
  summary: 400,
  intent: 120,
  note: 160,
  value: 200,
  text: 80,
  variable: 40,
  field: 80,
  kind: 24,
  caveat: 200,
} as const;

export const REPLAYABLE_ACTIONS = [
  'page.navigate',
  'page.clickElement',
  'page.fillInput',
  'page.selectOption',
  'page.pressKey',
  'page.submitForm',
  'page.scrollTo',
  'page.hoverElement',
  'page.focusInput',
  'page.selectText',
  'page.waitForElement',
  'page.extractText',
  'page.getPageInfo',
] as const;

export interface RecordingStep {
  ordinal: number;
  intent: string;
  action: string;
  target?: { selector?: string; text?: string; role?: string };
  value?: string;
  url?: string;
  note?: string;
}

export interface RecordingVariable {
  name: string;
  field: string;
  kind: string;
}

export interface RecordingWorkflow {
  goal: string;
  summary: string;
  steps: RecordingStep[];
  variables: RecordingVariable[];
  caveats: string[];
}

export type WorkflowValidation =
  | { ok: true; workflow: RecordingWorkflow; warnings: string[] }
  | { ok: false; message: string };

/**
 * `vexdom-pure`'s check (through ext-core), the same one the Bridge runs on the analyst's answer.
 * Synchronous, so ext-core must be loaded (`loadCore()`) first.
 */
export function validateRecordingWorkflow(input: unknown, opts: { origin: string }): WorkflowValidation {
  return JSON.parse(workflowValidate(jsonText(input), opts.origin)) as WorkflowValidation;
}
