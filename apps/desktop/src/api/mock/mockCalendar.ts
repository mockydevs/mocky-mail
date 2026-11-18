import type { CalendarEvent, CalendarInfo, EventInput } from '../types';
import { at } from './mockData';

const DAY = 86_400;

export const MOCK_CALENDARS: CalendarInfo[] = [
  { id: 'work:personal', accountId: 'work', name: 'Personal Calendar', color: '#2563EBFF' },
  { id: 'personal:personal', accountId: 'personal', name: 'Family', color: '#0F9D58FF' },
];

/** A stored event (its first date) plus dates removed from its series. */
export interface MockEvent {
  event: CalendarEvent;
  skipped: number[];
}

function event(partial: Partial<CalendarEvent> & Pick<CalendarEvent, 'id' | 'title' | 'start' | 'end'>): CalendarEvent {
  const calendar = MOCK_CALENDARS.find((c) => c.id === (partial.calendarId ?? 'work:personal')) ?? MOCK_CALENDARS[0];
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
    ...partial,
  };
}

/** Sample events around Fri 13 Nov 2026 (times are Nairobi, UTC+3). */
export const MOCK_EVENTS: MockEvent[] = [
  {
    event: event({
      id: 'e-standup',
      title: 'Team stand-up',
      start: at('2026-11-09T06:00:00Z'),
      end: at('2026-11-09T06:15:00Z'),
      repeat: 'daily',
      recurring: true,
      location: 'Google Meet',
    }),
    skipped: [at('2026-11-14T06:00:00Z'), at('2026-11-15T06:00:00Z')],
  },
  {
    event: event({
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
    event: event({
      id: 'e-launch',
      title: 'Website launch',
      start: at('2026-11-18T00:00:00Z'),
      end: at('2026-11-20T00:00:00Z'),
      allDay: true,
    }),
    skipped: [],
  },
  {
    event: event({
      id: 'e-dinner',
      calendarId: 'personal:personal',
      title: 'Family dinner',
      start: at('2026-11-15T15:30:00Z'),
      end: at('2026-11-15T18:00:00Z'),
      repeat: 'weekly',
      recurring: true,
    }),
    skipped: [],
  },
];

function step(start: number, repeat: CalendarEvent['repeat'], n: number): number {
  if (repeat === 'daily') return start + n * DAY;
  if (repeat === 'weekly') return start + n * 7 * DAY;
  const d = new Date(start * 1000);
  if (repeat === 'monthly') d.setUTCMonth(d.getUTCMonth() + n);
  else d.setUTCFullYear(d.getUTCFullYear() + n);
  return Math.floor(d.getTime() / 1000);
}

/** The occurrences of stored events between two times, like the backend. */
export function mockOccurrences(events: readonly MockEvent[], from: number, to: number): CalendarEvent[] {
  const out: CalendarEvent[] = [];
  for (const { event: e, skipped } of events) {
    const length = e.end - e.start;
    const overlaps = (s: number) => s < to && (s + length > from || (length === 0 && s >= from));
    if (!e.recurring || e.repeat === 'none' || e.repeat === 'custom') {
      if (overlaps(e.start)) out.push(e);
      continue;
    }
    for (let n = 0; n < 1000; n++) {
      const s = step(e.start, e.repeat, n);
      if (s >= to) break;
      if (overlaps(s) && !skipped.includes(s)) out.push({ ...e, start: s, end: s + length, recurrenceId: s });
    }
  }
  return out.sort((a, b) => Number(b.allDay) - Number(a.allDay) || a.start - b.start || a.end - b.end);
}

/** An event saved from the editor. */
export function mockSave(old: CalendarEvent | undefined, input: EventInput, id: string): CalendarEvent {
  const calendar = MOCK_CALENDARS.find((c) => c.id === input.calendarId);
  const repeat = input.repeat === 'keep' ? (old?.repeat ?? 'none') : input.repeat;
  return {
    ...(old ?? event({ id, title: '', start: 0, end: 0 })),
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
    recurring: repeat !== 'none',
    recurrenceId: null,
  };
}
