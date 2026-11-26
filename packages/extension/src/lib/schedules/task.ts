import type { TokenUsage } from '@/lib/actions/protocol';
import {
  jsonText,
  localTimeZone,
  scheduleHeadlineOf,
  scheduleNextRunFor,
  scheduleOrderFor,
  scheduleValidateRule,
  scheduleValidateTask,
} from '@/lib/bridge/pure';
import type { ScheduleRule } from './rule';

export const MAX_TASKS = 25;
export const MIN_EVERY_MINUTES = 5;
export const MAX_EVERY_MINUTES = 7 * 24 * 60;
export const MAX_TIMES_PER_DAY = 12;
export const MAX_RUNS_KEPT = 20;
export const MAX_TASK_RUNS = 1_000;
export const MAX_TASK_NAME = 80;
export const MAX_INSTRUCTION_CHARS = 2_000;
export const MAX_VARIABLES = 20;
export const MAX_VARIABLE_CHARS = 200;
export const MAX_HEADLINE_CHARS = 240;
export const TASK_APPROVAL_WAIT_MS = 10 * 60_000;
export const TASK_RUN_MAX_MS = 20 * 60_000;

export type TaskJob =
  | { kind: 'instruction'; text: string }
  | { kind: 'recording'; recordingId: string; name: string; variables?: Record<string, string> };

export type MissedPolicy = 'runOnce' | 'skip';

export type NotifyPolicy = 'always' | 'failure' | 'never';

export type RunOutcome = 'ok' | 'failed' | 'missed' | 'skipped' | 'cancelled';

export interface TaskRun {
  at: number;
  /** Which dispatch this was, so a report that arrives after the daemon gave up replaces that entry. */
  runId?: string;
  outcome: RunOutcome;
  reason?: string;
  headline?: string;
  durationMs?: number;
  usage?: TokenUsage;
  sessionId?: string;
}

export interface TaskDraft {
  id?: string;
  name: string;
  job: TaskJob;
  url: string;
  browser?: string;
  rule: ScheduleRule;
  missed: MissedPolicy;
  notify: NotifyPolicy;
  keepTab: boolean;
  maxRuns?: number;
  until?: number;
  enabled: boolean;
}

export interface ScheduledTask extends Omit<TaskDraft, 'id'> {
  id: string;
  nextRunAt: number | null;
  createdAt: number;
  runCount: number;
  runs: TaskRun[];
}

export interface TaskList {
  paused: boolean;
  tasks: ScheduledTask[];
  running: Record<string, number>;
}

export interface TaskOrder {
  id: string;
  runId: string;
  name: string;
  job: TaskJob;
  url: string;
  keepTab: boolean;
  notify: NotifyPolicy;
  previous?: string;
}

export interface TaskContext {
  id: string;
  name: string;
  previous?: string;
}

export type TaskResult = Omit<TaskRun, 'at'>;

export const EMPTY_TASK_LIST: TaskList = { paused: false, tasks: [], running: {} };

export type TaskValidation = { ok: true; draft: TaskDraft } | { ok: false; field: string; message: string };

/**
 * Validation, run times and orders come from `vexdom-pure` (through ext-core), the same code the
 * Bridge stores tasks with. Synchronous, so ext-core must be loaded (`loadCore()`) first.
 */

/** A time already in the past is refused only when it is new: a task that has had its day can still be renamed or switched off. */
export function validateRule(raw: unknown, now: number, stored?: ScheduleRule): ScheduleRule | string {
  const checked = JSON.parse(scheduleValidateRule(jsonText(raw), now, stored === undefined ? null : jsonText(stored))) as
    | { ok: true; rule: ScheduleRule }
    | { ok: false; message: string };
  return checked.ok ? checked.rule : checked.message;
}

export function validateTask(input: unknown, now: number, stored?: Pick<ScheduledTask, 'rule' | 'until'>): TaskValidation {
  return JSON.parse(scheduleValidateTask(jsonText(input), now, stored === undefined ? null : jsonText(stored))) as TaskValidation;
}

export function nextRunFor(
  task: Pick<ScheduledTask, 'rule' | 'enabled' | 'until' | 'maxRuns' | 'runCount'>,
  after: number,
): number | null {
  return scheduleNextRunFor(jsonText(task), after, localTimeZone());
}

export const orderFor = (task: ScheduledTask, runId: string): TaskOrder => JSON.parse(scheduleOrderFor(jsonText(task), runId)) as TaskOrder;

export function headlineOf(text: string): string | undefined {
  return scheduleHeadlineOf(text) ?? undefined;
}
