import type { CalendarEvent, EventInput, RepeatRule, ShowAs } from '../../api/types';
import { entryFor, parseRecipients } from '../compose/recipients';
import { DAY, dayKey, deviceZone, eventDays, fromInputs, timeValue, utcMidnight, type DayKey } from './calendarTime';
import { weekdaysRule } from './repeatRule';

/** Reminder choices in the editor, minutes before (-1: none). */
export const REMINDERS = [-1, 0, 5, 10, 15, 30, 60, 120, 1440, 10080];

/** What the Repeat list holds: the presets, a rule from the Custom dialog, or a rule set elsewhere. */
export type RepeatChoice = 'none' | 'daily' | 'weekly' | 'weekdays' | 'monthly' | 'yearly' | 'rule' | 'keep';

/** What the editor holds: dates and times as the inputs show them. */
export interface Form {
  id?: string;
  calendarId: string;
  title: string;
  location: string;
  notes: string;
  allDay: boolean;
  startDay: DayKey;
  startTime: string;
  endDay: DayKey;
  endTime: string;
  repeat: RepeatChoice;
  /** The rule for `rule` (from the Custom dialog, or the series' own). */
  repeatRule: RepeatRule | null;
  /** The series' rule as loaded: unchanged, it is kept as it is. */
  initialRule: RepeatRule | null;
  /** Minutes before, each once. */
  reminders: number[];
  showAs: ShowAs;
  private: boolean;
  /** People invited, as recipient-field text. */
  attendees: string;
  /** Addresses invited before this edit. */
  invited: string[];
  /** Editing one date of a series on its own: its recurrenceId. */
  occurrence: number | null;
}

const sameRule = (a: RepeatRule | null, b: RepeatRule | null) => JSON.stringify(a) === JSON.stringify(b);

/** The Repeat choice that shows an event's rule. */
function repeatChoice(e: CalendarEvent): RepeatChoice {
  if (e.repeatRule && sameRule(e.repeatRule, weekdaysRule())) return 'weekdays';
  if (e.repeat === 'custom') return e.repeatRule ? 'rule' : 'keep';
  if (e.repeatRule && (e.repeatRule.interval > 1 || e.repeatRule.count !== null || e.repeatRule.until !== null)) return 'rule';
  return e.repeat;
}

/** The editor's copy of an event (`own`: your addresses, left out of the people invited). */
export function formFrom(e: CalendarEvent, own: readonly string[] = [], occurrence: number | null = null): Form {
  const [first, last] = eventDays(e);
  const mine = new Set(own.map((a) => a.toLowerCase()));
  const people = e.attendees.filter((p) => !mine.has(p.email.toLowerCase()));
  return {
    id: e.id,
    calendarId: e.calendarId,
    title: e.title,
    location: e.location,
    notes: e.notes,
    allDay: e.allDay,
    startDay: dayKey(first),
    startTime: e.allDay ? '09:00' : timeValue(new Date(e.start * 1000)),
    endDay: dayKey(last),
    endTime: e.allDay ? '10:00' : timeValue(new Date(e.end * 1000)),
    repeat: occurrence !== null ? 'none' : repeatChoice(e),
    repeatRule: e.repeatRule,
    initialRule: e.repeatRule,
    reminders: [...new Set(e.reminders)].sort((a, b) => a - b),
    showAs: e.showAs,
    private: e.private,
    attendees: people.map((p) => entryFor({ name: p.name, address: p.email })).join(', '),
    invited: people.map((p) => p.email.toLowerCase()),
    occurrence,
  };
}

/** A new event on `day`, from `startMins` for `lengthMins` (default: the next whole hour today, else 9:00). */
export function newForm(
  day: Date,
  calendarId: string,
  now: Date,
  reminder: number,
  startMins?: number,
  lengthMins = 60,
  allDay = false,
): Form {
  const isToday = dayKey(day) === dayKey(now);
  const start = startMins ?? (isToday ? Math.min(now.getHours() + 1, 23) : 9) * 60;
  const end = Math.min(start + lengthMins, 24 * 60 - 1);
  const pad = (n: number) => String(n).padStart(2, '0');
  const at = (mins: number) => `${pad(Math.floor(mins / 60))}:${pad(mins % 60)}`;
  return {
    calendarId,
    title: '',
    location: '',
    notes: '',
    allDay,
    startDay: dayKey(day),
    startTime: at(start),
    endDay: dayKey(day),
    endTime: at(end),
    repeat: 'none',
    repeatRule: null,
    initialRule: null,
    reminders: reminder < 0 ? [] : [reminder],
    showAs: allDay ? 'free' : 'busy',
    private: false,
    attendees: '',
    invited: [],
    occurrence: null,
  };
}

/** Start and end of the form's times, as Unix seconds. */
export function formTimes(form: Form): { start: number; end: number } {
  const start = form.allDay ? utcMidnight(form.startDay) : fromInputs(form.startDay, form.startTime);
  const end = form.allDay ? utcMidnight(form.endDay) + DAY : fromInputs(form.endDay, form.endTime);
  return { start, end };
}

/** The addresses invited in the form, or the text that isn't one. */
export function attendeesOf(form: Form): { emails: string[]; invalid: string[] } {
  const parsed = parseRecipients(form.attendees);
  return { emails: parsed.valid, invalid: parsed.invalid };
}

/**
 * The event to save from the form, or why it can't be. `editable` false: a
 * read-only event, where only your reminders change.
 */
export function toInput(form: Form, editable = true): EventInput | string {
  if (!form.startDay || !form.endDay) return 'Choose a date for the event.';
  const { start, end } = formTimes(form);
  if (end < start || (form.allDay && end <= start)) return 'The event ends before it starts.';
  const people = attendeesOf(form);
  if (editable && people.invalid.length > 0) return `Not a valid address: ${people.invalid.join(', ')}`;
  const input: EventInput = {
    ...(form.id ? { id: form.id } : {}),
    calendarId: form.calendarId,
    title: form.title,
    location: form.location,
    notes: form.notes,
    allDay: form.allDay,
    start,
    end,
    repeat: 'keep',
    timeZone: deviceZone(),
    reminders: form.reminders,
  };
  if (!editable) return input;
  input.showAs = form.showAs;
  input.private = form.private;
  input.attendees = people.emails;
  if (form.occurrence !== null) {
    input.occurrence = form.occurrence;
    input.scope = 'this';
    input.repeat = 'none';
    return input;
  }
  switch (form.repeat) {
    case 'keep':
      break;
    case 'rule':
      if (form.repeatRule && !(form.id && sameRule(form.repeatRule, form.initialRule))) {
        input.repeat = 'none';
        input.repeatRule = form.repeatRule;
      }
      break;
    case 'weekdays':
      input.repeat = 'weekly';
      input.repeatRule = weekdaysRule();
      break;
    default:
      input.repeat = form.repeat;
      input.repeatRule = null;
  }
  if (form.id) input.scope = 'all';
  return input;
}

/** The input that changes an event's times (dragging, resizing), keeping everything else. */
export function movedInput(e: CalendarEvent, start: number, end: number): EventInput {
  return {
    id: e.id,
    calendarId: e.calendarId,
    title: e.title,
    location: e.location,
    notes: e.notes,
    allDay: e.allDay,
    start,
    end,
    repeat: 'keep',
    timeZone: deviceZone(),
    reminders: null,
  };
}

/** A copy of an event at other times (Ctrl+drag): a new single event without the people. */
export function copiedInput(e: CalendarEvent, start: number, end: number): EventInput {
  const moved: EventInput = movedInput(e, start, end);
  delete moved.id;
  return { ...moved, repeat: 'none', reminders: e.reminders, showAs: e.showAs, private: e.private };
}
