import type { CalendarEvent, CalendarInfo, EventInput, EventPerson, RepeatRule } from '../types';
import { at } from './mockData';

const DAY = 86_400;

export const MOCK_CALENDARS: CalendarInfo[] = [
  {
    id: 'work:personal',
    accountId: 'work',
    name: 'Personal Calendar',
    color: '#2563EBFF',
    visible: true,
    writable: true,
    provider: 'caldav',
  },
  {
    id: 'work:holidays',
    accountId: 'work',
    name: 'Kenya holidays',
    color: '#B5471BFF',
    visible: true,
    writable: false,
    provider: 'caldav',
  },
  {
    id: 'personal:personal',
    accountId: 'personal',
    name: 'Family',
    color: '#0F9D58FF',
    visible: true,
    writable: true,
    provider: 'caldav',
  },
];

/** A stored event (its first date) plus dates removed from its series. */
export interface MockEvent {
  event: CalendarEvent;
  skipped: number[];
}

export function mockEvent(
  partial: Partial<CalendarEvent> & Pick<CalendarEvent, 'id' | 'title' | 'start' | 'end'>,
  calendars: readonly CalendarInfo[] = MOCK_CALENDARS,
): CalendarEvent {
  const calendar = calendars.find((c) => c.id === (partial.calendarId ?? 'work:personal')) ?? calendars[0];
  return {
    calendarId: calendar?.id ?? 'work:personal',
    accountId: calendar?.accountId ?? 'work',
    color: calendar?.color ?? null,
    uid: `${partial.id}@mock`,
    recurrenceId: null,
    location: '',
    notes: '',
    allDay: false,
    repeat: 'none',
    recurring: false,
    organizer: null,
    attendees: [],
    tentative: false,
    reminders: [15],
    showAs: 'busy',
    private: false,
    repeatRule: null,
    isOrganizer: true,
    ...partial,
  };
}

/** Sample events around Fri 13 Nov 2026 (times are Nairobi, UTC+3). */
export const MOCK_EVENTS: MockEvent[] = [
  {
    event: mockEvent({
      id: 'e-standup',
      title: 'Team stand-up',
      start: at('2026-11-09T06:00:00Z'),
      end: at('2026-11-09T06:15:00Z'),
      repeat: 'daily',
      recurring: true,
      repeatRule: rule('daily'),
      location: 'Google Meet',
    }),
    skipped: [at('2026-11-14T06:00:00Z'), at('2026-11-15T06:00:00Z')],
  },
  {
    event: mockEvent({
      id: 'e-review',
      title: 'Design review with Acme',
      start: at('2026-11-13T09:00:00Z'),
      end: at('2026-11-13T10:30:00Z'),
      location: 'Westlands office, room 2',
      notes: 'Bring the printed mock-ups.',
      organizer: { email: 'you@work.example', name: 'You', status: 'accepted' },
      attendees: [
        { email: 'peter@acme.example', name: 'Peter Mwangi', status: 'accepted' },
        { email: 'amina@acme.example', name: 'Amina Yusuf', status: 'needs-action' },
      ],
    }),
    skipped: [],
  },
  {
    event: mockEvent({
      id: 'e-supplier',
      title: 'Supplier call',
      start: at('2026-11-17T08:00:00Z'),
      end: at('2026-11-17T08:30:00Z'),
      location: 'Phone',
      organizer: { email: 'peter@acme.example', name: 'Peter Mwangi', status: 'accepted' },
      attendees: [{ email: 'you@work.example', name: 'You', status: 'tentative' }],
      tentative: true,
      showAs: 'tentative',
      isOrganizer: false,
    }),
    skipped: [],
  },
  {
    event: mockEvent({
      id: 'e-launch',
      title: 'Website launch',
      start: at('2026-11-18T00:00:00Z'),
      end: at('2026-11-20T00:00:00Z'),
      allDay: true,
      showAs: 'free',
    }),
    skipped: [],
  },
  {
    event: mockEvent({
      id: 'e-report',
      title: 'Monthly report',
      start: at('2026-11-27T11:00:00Z'),
      end: at('2026-11-27T12:00:00Z'),
      repeat: 'custom',
      recurring: true,
      repeatRule: { ...rule('monthly'), byDay: ['FR'], setPos: -1 },
    }),
    skipped: [],
  },
  {
    event: mockEvent({
      id: 'e-dinner',
      calendarId: 'personal:personal',
      title: 'Family dinner',
      start: at('2026-11-15T15:30:00Z'),
      end: at('2026-11-15T18:00:00Z'),
      repeat: 'weekly',
      recurring: true,
      repeatRule: { ...rule('weekly'), byDay: ['SU'] },
    }),
    skipped: [],
  },
  {
    event: mockEvent({
      id: 'e-jamhuri',
      calendarId: 'work:holidays',
      title: 'Jamhuri Day',
      start: at('2026-12-12T00:00:00Z'),
      end: at('2026-12-13T00:00:00Z'),
      allDay: true,
      showAs: 'free',
      reminders: [],
      isOrganizer: false,
    }),
    skipped: [],
  },
];

function rule(freq: RepeatRule['freq']): RepeatRule {
  return { freq, interval: 1, byDay: [], monthDay: null, month: null, setPos: null, count: null, until: null };
}

const CODES = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'];

/** The rule a simple repeat stands for, from the first date. */
export function ruleOf(e: Pick<CalendarEvent, 'repeat' | 'repeatRule' | 'start' | 'allDay'>): RepeatRule | null {
  if (e.repeatRule) return e.repeatRule;
  const d = dateOf(e.start, e.allDay);
  switch (e.repeat) {
    case 'daily':
      return rule('daily');
    case 'weekly':
      return { ...rule('weekly'), byDay: [CODES[d.getDay()] ?? 'MO'] };
    case 'monthly':
      return { ...rule('monthly'), monthDay: d.getDate() };
    case 'yearly':
      return { ...rule('yearly'), month: d.getMonth() + 1, monthDay: d.getDate() };
    default:
      return null;
  }
}

/** The calendar day an event starts on: local for timed events, the UTC date for all-day ones. */
function dateOf(secs: number, allDay: boolean): Date {
  const d = new Date(secs * 1000);
  return allDay ? new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()) : new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

/** Whether `day` is the `pos`th (or last, -1) such weekday of its month. */
function nthWeekday(day: Date, pos: number): boolean {
  if (pos > 0) return Math.floor((day.getDate() - 1) / 7) + 1 === pos;
  const next = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 7);
  return pos === -1 && next.getMonth() !== day.getMonth();
}

function dayMatches(r: RepeatRule, first: Date, day: Date): boolean {
  const days = Math.round((day.getTime() - first.getTime()) / 86_400_000);
  const interval = Math.max(1, r.interval);
  const code = CODES[day.getDay()] ?? '';
  const months = (day.getFullYear() - first.getFullYear()) * 12 + day.getMonth() - first.getMonth();
  const byDate = (monthDay: number) => day.getDate() === monthDay;
  const byPos = () => r.setPos !== null && r.byDay.includes(code) && nthWeekday(day, r.setPos);
  switch (r.freq) {
    case 'daily':
      return days % interval === 0;
    case 'weekly': {
      // Weeks counted from the Monday of the first week.
      const monday = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate() - ((d.getDay() + 6) % 7));
      const weeks = Math.round((monday(day).getTime() - monday(first).getTime()) / (7 * 86_400_000));
      const codes = r.byDay.length > 0 ? r.byDay : [CODES[first.getDay()] ?? 'MO'];
      return weeks % interval === 0 && codes.includes(code);
    }
    case 'monthly':
      if (months % interval !== 0) return false;
      return r.setPos !== null ? byPos() : byDate(r.monthDay ?? first.getDate());
    case 'yearly':
      if ((day.getFullYear() - first.getFullYear()) % interval !== 0) return false;
      if (day.getMonth() + 1 !== (r.month ?? first.getMonth() + 1)) return false;
      return r.setPos !== null ? byPos() : byDate(r.monthDay ?? first.getDate());
  }
}

/** The starts of a series between its first date and `to`, honouring the rule (count, until). */
export function seriesStarts(e: CalendarEvent, to: number): number[] {
  const r = ruleOf(e);
  if (!e.recurring || !r) return [e.start];
  const first = dateOf(e.start, e.allDay);
  const startDate = new Date(e.start * 1000);
  const out: number[] = [];
  for (let n = 0; n < 4000; n++) {
    const day = new Date(first.getFullYear(), first.getMonth(), first.getDate() + n);
    const s = e.allDay
      ? Date.UTC(day.getFullYear(), day.getMonth(), day.getDate()) / 1000
      : new Date(
          day.getFullYear(),
          day.getMonth(),
          day.getDate(),
          startDate.getHours(),
          startDate.getMinutes(),
        ).getTime() / 1000;
    if (s >= to) break;
    if (r.until !== null && s > r.until + DAY - 1) break;
    if (!dayMatches(r, first, day)) continue;
    out.push(s);
    if (r.count !== null && out.length >= r.count) break;
  }
  return out;
}

/** The occurrences of stored events between two times, like the backend. */
export function mockOccurrences(events: readonly MockEvent[], from: number, to: number): CalendarEvent[] {
  const out: CalendarEvent[] = [];
  for (const { event: e, skipped } of events) {
    const length = e.end - e.start;
    const overlaps = (s: number) => s < to && (s + length > from || (length === 0 && s >= from));
    if (!e.recurring || e.repeat === 'none') {
      if (overlaps(e.start)) out.push(e);
      continue;
    }
    for (const s of seriesStarts(e, to)) {
      if (overlaps(s) && !skipped.includes(s)) out.push({ ...e, start: s, end: s + length, recurrenceId: s });
    }
  }
  return out.sort((a, b) => Number(b.allDay) - Number(a.allDay) || a.start - b.start || a.end - b.end);
}

/** The simple repeat a rule matches, or `custom`. */
function repeatOf(r: RepeatRule, start: number, allDay: boolean): CalendarEvent['repeat'] {
  const simple = ruleOf({ repeat: r.freq, repeatRule: null, start, allDay });
  const same =
    simple !== null &&
    r.interval === 1 &&
    r.count === null &&
    r.until === null &&
    r.setPos === null &&
    JSON.stringify([...r.byDay].sort()) === JSON.stringify([...simple.byDay].sort()) &&
    (r.monthDay ?? simple.monthDay) === simple.monthDay &&
    (r.month ?? simple.month) === simple.month;
  return same ? r.freq : 'custom';
}

/** An event saved from the editor. */
export function mockSave(
  old: CalendarEvent | undefined,
  input: EventInput,
  id: string,
  calendars: readonly CalendarInfo[] = MOCK_CALENDARS,
  ownEmail = 'you@work.example',
): CalendarEvent {
  const calendar = calendars.find((c) => c.id === input.calendarId);
  let repeat: CalendarEvent['repeat'];
  let repeatRule: RepeatRule | null;
  if (input.repeatRule) {
    repeatRule = input.repeatRule;
    repeat = repeatOf(input.repeatRule, input.start, input.allDay);
  } else if (input.repeat === 'keep') {
    repeat = old?.repeat ?? 'none';
    repeatRule = old?.repeatRule ?? null;
  } else {
    repeat = input.repeat;
    repeatRule = ruleOf({ repeat, repeatRule: null, start: input.start, allDay: input.allDay });
  }
  const base = old ?? mockEvent({ id, title: '', start: 0, end: 0 }, calendars);
  let attendees: EventPerson[] = base.attendees;
  let organizer = base.organizer;
  if (input.attendees) {
    attendees = input.attendees.map(
      (email) =>
        base.attendees.find((p) => p.email.toLowerCase() === email.toLowerCase()) ?? {
          email,
          name: null,
          status: 'needs-action',
        },
    );
    organizer = attendees.length > 0 ? (organizer ?? { email: ownEmail, name: null, status: 'accepted' }) : null;
  }
  return {
    ...base,
    id,
    calendarId: input.calendarId,
    accountId: calendar?.accountId ?? old?.accountId ?? 'work',
    color: calendar?.color ?? null,
    title: input.title.trim(),
    location: input.location.trim(),
    notes: input.notes.trim(),
    allDay: input.allDay,
    start: input.start,
    end: input.end,
    repeat,
    repeatRule,
    recurring: repeat !== 'none',
    recurrenceId: null,
    reminders: input.reminders ?? old?.reminders ?? [],
    showAs: input.showAs ?? base.showAs,
    private: input.private ?? base.private,
    attendees,
    organizer,
  };
}
