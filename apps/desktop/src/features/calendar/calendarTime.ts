import type { CalendarEvent, Repeat } from '../../api/types';

export const DAY = 86_400;

/** A day as `YYYY-MM-DD`, the key events are grouped by. */
export type DayKey = string;

const pad = (n: number) => String(n).padStart(2, '0');

/** Local midnight of the day containing `date`. */
export function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

export function addDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

export function addMonths(date: Date, months: number): Date {
  return new Date(date.getFullYear(), date.getMonth() + months, 1);
}

export function dayKey(date: Date): DayKey {
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** `YYYY-MM-DD` to local midnight. */
export function fromDayKey(key: DayKey): Date {
  const [y = 1970, m = 1, d = 1] = key.split('-').map(Number);
  return new Date(y, m - 1, d);
}

export const seconds = (date: Date) => Math.floor(date.getTime() / 1000);

/** The six weeks shown for a month, Monday first. */
export function monthGrid(month: Date): Date[] {
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  const back = (first.getDay() + 6) % 7;
  const start = addDays(first, -back);
  return Array.from({ length: 42 }, (_, i) => addDays(start, i));
}

/** All-day dates are stored as midnight UTC: read them as calendar dates. */
function utcDay(secondsUtc: number): Date {
  const d = new Date(secondsUtc * 1000);
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
}

/** First and last local day an event covers. */
export function eventDays(e: CalendarEvent): [Date, Date] {
  if (e.allDay) {
    const first = utcDay(e.start);
    const last = utcDay(Math.max(e.start, e.end - DAY));
    return [first, last];
  }
  const first = startOfDay(new Date(e.start * 1000));
  // An event ending exactly at midnight doesn't show on the next day.
  const last = startOfDay(new Date(Math.max(e.start, e.end - 1) * 1000));
  return [first, last];
}

/** Events by the days they cover (at most 62 days each). */
export function byDay(events: readonly CalendarEvent[]): Map<DayKey, CalendarEvent[]> {
  const out = new Map<DayKey, CalendarEvent[]>();
  for (const e of events) {
    const [first, last] = eventDays(e);
    for (let d = first, n = 0; d <= last && n < 62; d = addDays(d, 1), n++) {
      const key = dayKey(d);
      const list = out.get(key);
      if (list) list.push(e);
      else out.set(key, [e]);
    }
  }
  return out;
}

const time = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
});
const dayLong = new Intl.DateTimeFormat(undefined, {
  weekday: 'short',
  day: 'numeric',
  month: 'short',
});
const dayYear = new Intl.DateTimeFormat(undefined, {
  weekday: 'short',
  day: 'numeric',
  month: 'short',
  year: 'numeric',
});

export function timeText(secondsUtc: number): string {
  return time.format(new Date(secondsUtc * 1000));
}

export function dayText(date: Date, now = new Date()): string {
  return (date.getFullYear() === now.getFullYear() ? dayLong : dayYear).format(date);
}

/** "Fri 13 Nov, 12:00 – 13:30", "Wed 18 – Thu 19 Nov, all day". */
export function whenText(e: CalendarEvent): string {
  const [first, last] = eventDays(e);
  if (e.allDay) {
    return first.getTime() === last.getTime()
      ? `${dayText(first)}, all day`
      : `${dayText(first)} – ${dayText(last)}, all day`;
  }
  const sameDay = first.getTime() === last.getTime();
  if (e.end === e.start) return `${dayText(first)}, ${timeText(e.start)}`;
  return sameDay
    ? `${dayText(first)}, ${timeText(e.start)} – ${timeText(e.end)}`
    : `${dayText(first)}, ${timeText(e.start)} – ${dayText(last)}, ${timeText(e.end)}`;
}

const weekday = new Intl.DateTimeFormat(undefined, { weekday: 'long' });

/** "Every week on Friday". */
export function repeatText(repeat: Repeat | 'keep', start: Date): string {
  switch (repeat) {
    case 'none':
      return "Doesn't repeat";
    case 'daily':
      return 'Every day';
    case 'weekly':
      return `Every week on ${weekday.format(start)}`;
    case 'monthly':
      return `Every month on day ${String(start.getDate())}`;
    case 'yearly':
      return 'Every year';
    case 'custom':
    case 'keep':
      return 'Repeats (as set elsewhere)';
  }
}

/** `<input type="time">` value. */
export function timeValue(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Local date and time inputs to Unix seconds. */
export function fromInputs(day: DayKey, hhmm: string): number {
  const date = fromDayKey(day);
  const [h = 0, m = 0] = hhmm.split(':').map(Number);
  date.setHours(h, m, 0, 0);
  return seconds(date);
}

/** A local date as stored for all-day events: midnight UTC. */
export function utcMidnight(day: DayKey): number {
  const [y = 1970, m = 1, d = 1] = day.split('-').map(Number);
  return Date.UTC(y, m - 1, d) / 1000;
}

/** This device's time zone, such as `Africa/Nairobi`. */
export function deviceZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return 'UTC';
  }
}

/** A server colour (`#RRGGBBAA`) as CSS, or the accent colour. SOGo's
 * default grey counts as no colour chosen. */
export function cssColor(color: string | null): string {
  if (color && /^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(color) && color.slice(0, 7).toUpperCase() !== '#AAAAAA') {
    return color.slice(0, 7);
  }
  return 'var(--primary)';
}

/** "15 minutes before", "1 day before". */
export function reminderText(mins: number): string {
  if (mins < 0) return 'No reminder';
  if (mins === 0) return 'At the start';
  if (mins % 10080 === 0) return mins === 10080 ? '1 week before' : `${String(mins / 10080)} weeks before`;
  if (mins % 1440 === 0) return mins === 1440 ? '1 day before' : `${String(mins / 1440)} days before`;
  if (mins % 60 === 0) return mins === 60 ? '1 hour before' : `${String(mins / 60)} hours before`;
  return `${String(mins)} minutes before`;
}
