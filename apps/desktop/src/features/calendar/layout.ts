import type { CalendarEvent } from '../../api/types';
import { minutesInto } from './calendarTime';

/** One event as placed in a day column of the time grid. */
export interface Placed {
  event: CalendarEvent;
  /** Minutes from midnight, clipped to the day. */
  top: number;
  /** Minutes long (at least 15, so short events stay clickable). */
  length: number;
  /** Column among the events it overlaps, from 0. */
  column: number;
  /** How many columns its overlapping group needs. */
  columns: number;
  /** It started before this day / goes on after it. */
  continuesBefore: boolean;
  continuesAfter: boolean;
}

const DAY_MINUTES = 24 * 60;
const MIN_LENGTH = 15;

/**
 * Lay out a day's timed events side by side where they overlap (as Outlook
 * does): events that overlap, directly or through others, form a group that
 * shares the column's width; each takes the first column free at its start.
 */
export function layoutDay(day: Date, events: readonly CalendarEvent[]): Placed[] {
  const items = events
    .filter((e) => !e.allDay)
    .map((event) => {
      const start = minutesInto(day, event.start);
      const end = minutesInto(day, event.end);
      const top = Math.max(0, start);
      const bottom = Math.min(DAY_MINUTES, Math.max(end, start + MIN_LENGTH));
      return {
        event,
        top,
        length: Math.max(MIN_LENGTH, bottom - top),
        continuesBefore: start < 0,
        continuesAfter: end > DAY_MINUTES,
      };
    })
    .filter((x) => x.top < DAY_MINUTES && x.top + x.length > 0)
    .sort((a, b) => a.top - b.top || b.length - a.length || a.event.title.localeCompare(b.event.title));

  const out: Placed[] = [];
  let group: Placed[] = [];
  /** End (minutes) of each column in the current group. */
  let ends: number[] = [];
  let groupEnd = -1;
  const close = () => {
    for (const p of group) p.columns = ends.length;
    group = [];
    ends = [];
  };
  for (const x of items) {
    if (x.top >= groupEnd) close();
    let column = ends.findIndex((end) => end <= x.top);
    if (column < 0) {
      column = ends.length;
      ends.push(0);
    }
    ends[column] = x.top + x.length;
    groupEnd = Math.max(groupEnd, x.top + x.length);
    const placed: Placed = { ...x, column, columns: 1 };
    group.push(placed);
    out.push(placed);
  }
  close();
  return out;
}
