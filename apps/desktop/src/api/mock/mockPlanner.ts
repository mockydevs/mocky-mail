import type { CalendarEvent, DueReminder, FreeBusy, ShowAs, Task, TaskList } from '../types';
import { at } from './mockData';

/** Sample to-do lists: one on this device, one on the work account's server. */
export const MOCK_TASK_LISTS: TaskList[] = [
  { id: 'device:tasks', accountId: null, name: 'Tasks', provider: 'device' },
  { id: 'work:tasks', accountId: 'work', name: 'Work', provider: 'caldav' },
];

const DAY_UTC = (iso: string) => at(`${iso}T00:00:00Z`);

/** Sample tasks around Fri 13 Nov 2026. */
export const MOCK_TASKS: Task[] = [
  {
    id: 't-quote',
    listId: 'work:tasks',
    title: 'Send the quotation to Acme',
    notes: 'Use the November price list.',
    due: DAY_UTC('2026-11-13'),
    reminder: null,
    importance: 'high',
    completed: false,
    completedAt: null,
    messageId: null,
    categories: ['Clients'],
    state: 'synced',
  },
  {
    id: 't-domain',
    listId: 'work:tasks',
    title: 'Renew the domain',
    notes: '',
    due: DAY_UTC('2026-11-10'),
    reminder: null,
    importance: 'normal',
    completed: false,
    completedAt: null,
    messageId: null,
    categories: [],
    state: 'synced',
  },
  {
    id: 't-paper',
    listId: 'device:tasks',
    title: 'Buy printer paper',
    notes: '',
    due: null,
    reminder: null,
    importance: 'low',
    completed: false,
    completedAt: null,
    messageId: null,
    categories: [],
    state: 'synced',
  },
  {
    id: 't-venue',
    listId: 'device:tasks',
    title: 'Book the venue',
    notes: '',
    due: DAY_UTC('2026-11-16'),
    reminder: null,
    importance: 'normal',
    completed: false,
    completedAt: null,
    messageId: null,
    categories: [],
    state: 'synced',
  },
  {
    id: 't-invoice',
    listId: 'work:tasks',
    title: 'Pay the October invoice',
    notes: '',
    due: DAY_UTC('2026-11-06'),
    reminder: null,
    importance: 'normal',
    completed: true,
    completedAt: at('2026-11-06T09:00:00Z'),
    messageId: null,
    categories: [],
    state: 'synced',
  },
];

/** Tasks in the order list_tasks returns them: open first, by due date (none last), then title. */
export function sortTasks(tasks: readonly Task[]): Task[] {
  return [...tasks].sort(
    (a, b) =>
      Number(a.completed) - Number(b.completed) ||
      (a.due ?? Number.MAX_SAFE_INTEGER) - (b.due ?? Number.MAX_SAFE_INTEGER) ||
      a.title.localeCompare(b.title),
  );
}

function hash(text: string): number {
  let h = 7;
  for (const ch of text) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h;
}

/**
 * Made-up but stable busy times for someone: a morning and an afternoon
 * meeting each weekday, plus any sample event they are invited to.
 */
export function mockFreeBusy(
  email: string,
  from: number,
  to: number,
  events: readonly CalendarEvent[],
  fixed?: Record<string, { start: number; end: number; showAs: ShowAs }[]>,
): FreeBusy {
  const key = email.trim().toLowerCase();
  if (fixed) {
    const busy = fixed[key];
    return busy ? { email, busy: [...busy].sort((a, b) => a.start - b.start), unknown: false } : { email, busy: [], unknown: true };
  }
  if (key.endsWith('.invalid') || key.includes('@unknown.')) return { email, busy: [], unknown: true };
  const h = hash(key);
  const busy: FreeBusy['busy'] = [];
  const first = new Date(from * 1000);
  for (let d = new Date(first.getFullYear(), first.getMonth(), first.getDate()); d.getTime() / 1000 < to; d.setDate(d.getDate() + 1)) {
    const weekday = d.getDay();
    if (weekday === 0 || weekday === 6) continue;
    const day = d.getTime() / 1000;
    const morning = day + (9 + ((h + weekday) % 3)) * 3600;
    const afternoon = day + (13 + ((h >> 3) + weekday) % 4) * 3600 + 1800;
    busy.push({ start: morning, end: morning + 3600, showAs: 'busy' });
    busy.push({ start: afternoon, end: afternoon + 5400, showAs: weekday === 5 ? 'tentative' : 'busy' });
  }
  for (const e of events) {
    if (e.showAs === 'free') continue;
    const invited = e.attendees.some((p) => p.email.toLowerCase() === key) || e.organizer?.email.toLowerCase() === key;
    if (invited) busy.push({ start: e.start, end: e.end, showAs: e.showAs });
  }
  return {
    email,
    busy: busy.filter((b) => b.end > from && b.start < to).sort((a, b) => a.start - b.start),
    unknown: false,
  };
}

export interface ReminderSources {
  now: number;
  /** Occurrences around now. */
  events: readonly CalendarEvent[];
  tasks: readonly Task[];
  flags: readonly { id: string; subject: string; flagDue: number | null; flagged: boolean }[];
  dismissed: ReadonlySet<string>;
  snoozed: ReadonlyMap<string, number>;
}

/** What is due now, like the backend's reminder clock. */
export function dueReminders(s: ReminderSources): DueReminder[] {
  const out: DueReminder[] = [];
  const blank = { location: null, eventId: null, recurrenceId: null, taskId: null, messageId: null };
  for (const e of s.events) {
    if (e.reminders.length === 0 || e.end <= s.now) continue;
    const earliest = Math.max(...e.reminders);
    if (e.start - earliest * 60 > s.now) continue;
    out.push({
      ...blank,
      key: `event:${e.id}:${String(e.start)}`,
      kind: 'event',
      title: e.title || '(No title)',
      at: e.start,
      location: e.location || null,
      eventId: e.id,
      recurrenceId: e.recurrenceId,
    });
  }
  for (const t of s.tasks) {
    if (t.completed || t.state === 'deleted' || t.reminder === null || t.reminder > s.now) continue;
    out.push({ ...blank, key: `task:${t.id}`, kind: 'task', title: t.title, at: t.due ?? t.reminder, taskId: t.id });
  }
  for (const m of s.flags) {
    if (!m.flagged || m.flagDue === null || m.flagDue > s.now) continue;
    out.push({ ...blank, key: `flag:${m.id}`, kind: 'flag', title: m.subject || '(no subject)', at: m.flagDue, messageId: m.id });
  }
  return out
    .filter((r) => !s.dismissed.has(r.key) && (s.snoozed.get(r.key) ?? 0) <= s.now)
    .sort((a, b) => a.at - b.at);
}
