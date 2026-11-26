import { jsonText, localTimeZone, scheduleDescribeMoment, scheduleDescribeRule, scheduleInsideWindow, scheduleNextAfter, scheduleUpcoming } from '@/lib/bridge/pure';

export type Weekday = 0 | 1 | 2 | 3 | 4 | 5 | 6;

export const WEEKDAYS: readonly Weekday[] = [1, 2, 3, 4, 5, 6, 0];

export const WORKDAYS: readonly Weekday[] = [1, 2, 3, 4, 5];

export const DAY_NAMES: Record<Weekday, string> = {
  0: 'Sun',
  1: 'Mon',
  2: 'Tue',
  3: 'Wed',
  4: 'Thu',
  5: 'Fri',
  6: 'Sat',
};

export interface TimeWindow {
  from: string;
  to: string;
}

export type ScheduleRule =
  | { kind: 'once'; at: number }
  | { kind: 'every'; minutes: number; window?: TimeWindow }
  | { kind: 'weekly'; days: Weekday[]; times: string[] };

interface ClockTime {
  hours: number;
  minutes: number;
}

const CLOCK_TIME = /^([01]\d|2[0-3]):([0-5]\d)$/;

export function parseClockTime(text: string): ClockTime | null {
  const match = CLOCK_TIME.exec(text);
  return match ? { hours: Number(match[1]), minutes: Number(match[2]) } : null;
}

export const isClockTime = (text: unknown): text is string => typeof text === 'string' && CLOCK_TIME.test(text);

export const isWeekday = (day: unknown): day is Weekday => Number.isInteger(day) && (day as number) >= 0 && (day as number) <= 6;

const pad = (value: number) => String(value).padStart(2, '0');

export const clockTimeOf = (at: number): string => {
  const date = new Date(at);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
};

/**
 * The rules run in `vexdom-pure` (through ext-core) on the browser's wall clock, the same code the
 * Bridge schedules with. Synchronous, so ext-core must be loaded (`loadCore()`) first.
 */
export function insideWindow(at: number, window: TimeWindow): boolean {
  return scheduleInsideWindow(at, jsonText(window), localTimeZone());
}

export function nextAfter(rule: ScheduleRule, after: number): number | null {
  return scheduleNextAfter(jsonText(rule), after, localTimeZone());
}

export function upcoming(rule: ScheduleRule, from: number, count: number): number[] {
  return [...scheduleUpcoming(jsonText(rule), from, count, localTimeZone())];
}

const sameSet = (a: readonly Weekday[], b: readonly Weekday[]) => a.length === b.length && b.every((day) => a.includes(day));

export function describeDays(days: readonly Weekday[]): string {
  if (sameSet(days, WEEKDAYS)) return 'Every day';
  if (sameSet(days, WORKDAYS)) return 'Weekdays';
  if (sameSet(days, [0, 6])) return 'Weekends';
  return WEEKDAYS.filter((day) => days.includes(day))
    .map((day) => DAY_NAMES[day])
    .join(', ');
}

export function joinList(items: readonly string[]): string {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`;
}

export function describeInterval(minutes: number): string {
  if (minutes % (24 * 60) === 0) return minutes === 24 * 60 ? 'Every day' : `Every ${minutes / (24 * 60)} days`;
  if (minutes % 60 === 0) return minutes === 60 ? 'Every hour' : `Every ${minutes / 60} h`;
  return `Every ${minutes} min`;
}

export function describeMoment(at: number): string {
  return scheduleDescribeMoment(at, localTimeZone());
}

export function describeRule(rule: ScheduleRule): string {
  return scheduleDescribeRule(jsonText(rule), localTimeZone());
}
