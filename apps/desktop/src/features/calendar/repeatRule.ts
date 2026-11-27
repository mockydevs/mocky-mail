import type { RepeatRule } from '../../api/types';
import { dayText } from './calendarTime';

/** RRULE weekday codes by `Date.getDay()`. */
export const DAY_CODES = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'] as const;

const weekdayLong = new Intl.DateTimeFormat(undefined, { weekday: 'long' });
const monthDay = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'long' });

/** "Monday" for `MO`. */
export function dayName(code: string): string {
  const index = DAY_CODES.indexOf(code as (typeof DAY_CODES)[number]);
  // 7 Jan 2024 was a Sunday.
  return weekdayLong.format(new Date(2024, 0, 7 + Math.max(0, index)));
}

const ORDINALS: Record<number, string> = { 1: 'first', 2: 'second', 3: 'third', 4: 'fourth', [-1]: 'last' };

export function ordinal(pos: number): string {
  return ORDINALS[pos] ?? `${String(pos)}th`;
}

/** Which of its weekday in the month a date is: 1–4, or -1 for the last (a fifth counts as last). */
export function weekdayPosition(date: Date): number {
  const n = Math.floor((date.getDate() - 1) / 7) + 1;
  const next = new Date(date.getFullYear(), date.getMonth(), date.getDate() + 7);
  return next.getMonth() !== date.getMonth() ? -1 : n;
}

export function blankRule(freq: RepeatRule['freq']): RepeatRule {
  return { freq, interval: 1, byDay: [], monthDay: null, month: null, setPos: null, count: null, until: null };
}

/** The rule for "Every weekday (Monday to Friday)". */
export function weekdaysRule(): RepeatRule {
  return { ...blankRule('weekly'), byDay: ['MO', 'TU', 'WE', 'TH', 'FR'] };
}

/** A sensible rule to start the Custom dialog with, from a start date. */
export function startingRule(freq: RepeatRule['freq'], start: Date): RepeatRule {
  switch (freq) {
    case 'daily':
      return blankRule('daily');
    case 'weekly':
      return { ...blankRule('weekly'), byDay: [DAY_CODES[start.getDay()] ?? 'MO'] };
    case 'monthly':
      return { ...blankRule('monthly'), monthDay: start.getDate() };
    case 'yearly':
      return { ...blankRule('yearly'), month: start.getMonth() + 1, monthDay: start.getDate() };
  }
}

const UNITS: Record<RepeatRule['freq'], [string, string]> = {
  daily: ['day', 'days'],
  weekly: ['week', 'weeks'],
  monthly: ['month', 'months'],
  yearly: ['year', 'years'],
};

function list(names: string[]): string {
  if (names.length <= 1) return names.join('');
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1] ?? ''}`;
}

/** "Every 2 weeks on Monday and Wednesday, 10 times". */
export function ruleText(rule: RepeatRule, start: Date): string {
  const [one, many] = UNITS[rule.freq];
  const every = rule.interval > 1 ? `Every ${String(rule.interval)} ${many}` : `Every ${one}`;
  let on = '';
  const week = DAY_CODES.filter((c) => rule.byDay.includes(c));
  if (rule.freq === 'weekly') {
    const weekdays = ['MO', 'TU', 'WE', 'TH', 'FR'];
    const isWeekdays = week.length === 5 && weekdays.every((c) => week.includes(c as (typeof DAY_CODES)[number]));
    // Monday first reads naturally in a list.
    const ordered = [...week.filter((c) => c !== 'SU'), ...week.filter((c) => c === 'SU')];
    on = isWeekdays && rule.interval === 1 ? '' : ` on ${list(ordered.map(dayName))}`;
    if (isWeekdays && rule.interval === 1) return withEnd('Every weekday (Monday to Friday)', rule);
    if (ordered.length === 0) on = ` on ${weekdayLong.format(start)}`;
  } else if (rule.freq === 'monthly' || rule.freq === 'yearly') {
    const monthName =
      rule.freq === 'yearly'
        ? ` of ${new Intl.DateTimeFormat(undefined, { month: 'long' }).format(new Date(2024, (rule.month ?? start.getMonth() + 1) - 1, 1))}`
        : '';
    if (rule.setPos !== null && week[0]) {
      on = ` on the ${ordinal(rule.setPos)} ${dayName(week[0])}${monthName}`;
    } else if (rule.freq === 'monthly') {
      on = ` on day ${String(rule.monthDay ?? start.getDate())}`;
    } else {
      const month = (rule.month ?? start.getMonth() + 1) - 1;
      on = ` on ${monthDay.format(new Date(2024, month, rule.monthDay ?? start.getDate()))}`;
    }
  }
  return withEnd(`${every}${on}`, rule);
}

function withEnd(text: string, rule: RepeatRule): string {
  if (rule.count !== null) return `${text}, ${String(rule.count)} time${rule.count === 1 ? '' : 's'}`;
  if (rule.until !== null) {
    const d = new Date(rule.until * 1000);
    return `${text}, until ${dayText(new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()))}`;
  }
  return text;
}
