/** Flag due dates: the quick choices in the flag menu, and how a due date reads. */

export type DueChoice = 'today' | 'tomorrow' | 'thisWeek' | 'nextWeek';

export const DUE_CHOICES: readonly { id: DueChoice; label: string }[] = [
  { id: 'today', label: 'Today' },
  { id: 'tomorrow', label: 'Tomorrow' },
  { id: 'thisWeek', label: 'This week' },
  { id: 'nextWeek', label: 'Next week' },
];

/** A flag falls due at the end of the working day. */
const DUE_HOUR = 17;

function seconds(date: Date): number {
  return Math.floor(date.getTime() / 1000);
}

function endOfWorkday(date: Date): Date {
  const out = new Date(date);
  out.setHours(DUE_HOUR, 0, 0, 0);
  return out;
}

/**
 * When a quick choice falls due, in local time: today (or tonight, once the
 * working day is over), tomorrow, Friday this week (today at the weekend),
 * or Monday next week.
 */
export function dueFor(choice: DueChoice, now: number): number {
  const today = new Date(now * 1000);
  const day = today.getDay();
  const shifted = (days: number) => {
    const d = new Date(today);
    d.setDate(d.getDate() + days);
    return endOfWorkday(d);
  };
  switch (choice) {
    case 'today': {
      const due = endOfWorkday(today);
      if (due.getTime() > today.getTime()) return seconds(due);
      const late = new Date(today);
      late.setHours(23, 59, 0, 0);
      return seconds(late);
    }
    case 'tomorrow':
      return seconds(shifted(1));
    case 'thisWeek':
      if (day === 0 || day === 6) return dueFor('today', now);
      return seconds(shifted(5 - day));
    case 'nextWeek':
      return seconds(shifted((8 - day) % 7 || 7));
  }
}

/** A date picked in a date field (`YYYY-MM-DD`), due at the end of that working day; null if not a date. */
export function dueFromDate(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]), DUE_HOUR, 0, 0, 0);
  return Number.isNaN(date.getTime()) ? null : seconds(date);
}

/** `YYYY-MM-DD` of a due date in local time, for a date field. */
export function dateValue(due: number): string {
  const d = new Date(due * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function isOverdue(due: number, now: number): boolean {
  return due < now;
}

function dayNumber(d: Date): number {
  return Math.round(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86_400_000);
}

/** "today", "tomorrow", "yesterday", "Mon 16 Nov" or "16 Nov 2027". */
export function dueDay(due: number, now: number): string {
  const d = new Date(due * 1000);
  const n = new Date(now * 1000);
  const diff = dayNumber(d) - dayNumber(n);
  if (diff === 0) return 'today';
  if (diff === 1) return 'tomorrow';
  if (diff === -1) return 'yesterday';
  const short = new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short' }).format(d);
  if (d.getFullYear() === n.getFullYear()) return short.replace(',', '');
  return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' }).format(d);
}

/** "Due tomorrow", or "Overdue: due yesterday". */
export function dueLabel(due: number, now: number): string {
  const day = dueDay(due, now);
  return isOverdue(due, now) ? `Overdue: due ${day}` : `Due ${day}`;
}
