import { mockEvent, mockOccurrences } from '../../api/mock/mockCalendar';
import { createMockMailApi } from '../../api/mock/mockMailApi';
import type { CalendarEvent, RepeatRule } from '../../api/types';
import { fitWithin } from '../contacts/photo';
import { dayKey, viewDays } from './calendarTime';
import { copiedInput, formFrom, newForm, toInput } from './eventForm';
import { layoutDay } from './layout';
import { blankRule, ruleText, weekdayPosition } from './repeatRule';

/** Unix seconds for a local time. */
const local = (m: number, d: number, h = 0, min = 0) => new Date(2026, m - 1, d, h, min).getTime() / 1000;

const ev = (id: string, start: number, end: number, extra: Partial<CalendarEvent> = {}) =>
  mockEvent({ id, title: id, start, end, ...extra });

describe('time grid layout', () => {
  const day = new Date(2026, 10, 13);

  it('puts overlapping events side by side and others full width', () => {
    const placed = layoutDay(day, [
      ev('a', local(11, 13, 9), local(11, 13, 10)),
      ev('b', local(11, 13, 9, 30), local(11, 13, 11)),
      ev('c', local(11, 13, 10, 30), local(11, 13, 11, 30)),
      ev('d', local(11, 13, 14), local(11, 13, 15)),
    ]);
    const by = Object.fromEntries(placed.map((p) => [p.event.id, p]));
    expect(by['a']).toMatchObject({ top: 540, length: 60, column: 0, columns: 2 });
    expect(by['b']).toMatchObject({ column: 1, columns: 2 });
    // c starts after a ends, so it takes a's column back.
    expect(by['c']).toMatchObject({ column: 0, columns: 2 });
    expect(by['d']).toMatchObject({ column: 0, columns: 1 });
  });

  it('clips events that run over midnight and keeps short ones clickable', () => {
    const [late] = layoutDay(day, [ev('late', local(11, 13, 23), local(11, 14, 1))]);
    expect(late).toMatchObject({ top: 1380, length: 60, continuesAfter: true, continuesBefore: false });
    const [early] = layoutDay(new Date(2026, 10, 14), [ev('late', local(11, 13, 23), local(11, 14, 1))]);
    expect(early).toMatchObject({ top: 0, length: 60, continuesBefore: true });
    const [tiny] = layoutDay(day, [ev('tiny', local(11, 13, 8), local(11, 13, 8, 5))]);
    expect(tiny?.length).toBe(15);
    expect(layoutDay(day, [ev('all', local(11, 13), local(11, 14), { allDay: true })])).toEqual([]);
  });

  it('works out the days of each view', () => {
    const friday = new Date(2026, 10, 13);
    expect(viewDays('day', friday).map(dayKey)).toEqual(['2026-11-13']);
    expect(viewDays('workweek', friday).map(dayKey)).toEqual([
      '2026-11-09',
      '2026-11-10',
      '2026-11-11',
      '2026-11-12',
      '2026-11-13',
    ]);
    // A Saturday shows the next work week.
    expect(viewDays('workweek', new Date(2026, 10, 14))[0]?.getDate()).toBe(16);
    expect(viewDays('week', friday, 1).map((d) => d.getDate())).toEqual([9, 10, 11, 12, 13, 14, 15]);
    expect(viewDays('week', friday, 0).map((d) => d.getDate())).toEqual([8, 9, 10, 11, 12, 13, 14]);
  });
});

describe('repeat rules', () => {
  const start = new Date(2026, 10, 13);

  it('describes custom rules', () => {
    expect(ruleText({ ...blankRule('daily'), interval: 3 }, start)).toBe('Every 3 days');
    expect(ruleText({ ...blankRule('weekly'), interval: 2, byDay: ['WE', 'MO'] }, start)).toBe(
      'Every 2 weeks on Monday and Wednesday',
    );
    expect(ruleText({ ...blankRule('weekly'), byDay: ['MO', 'TU', 'WE', 'TH', 'FR'] }, start)).toBe(
      'Every weekday (Monday to Friday)',
    );
    expect(ruleText({ ...blankRule('monthly'), byDay: ['TU'], setPos: 2 }, start)).toBe('Every month on the second Tuesday');
    expect(ruleText({ ...blankRule('monthly'), monthDay: 26, count: 10 }, start)).toBe('Every month on day 26, 10 times');
    expect(ruleText({ ...blankRule('yearly'), byDay: ['FR'], setPos: -1, month: 11 }, start)).toBe(
      'Every year on the last Friday of November',
    );
  });

  it('knows which weekday of the month a date is', () => {
    expect(weekdayPosition(new Date(2026, 10, 13))).toBe(2);
    expect(weekdayPosition(new Date(2026, 10, 27))).toBe(-1);
  });

  it('shows occurrences of custom rules in the mock backend', () => {
    const lastFriday: RepeatRule = { ...blankRule('monthly'), byDay: ['FR'], setPos: -1 };
    const report = ev('r', local(11, 27, 14), local(11, 27, 15), { recurring: true, repeat: 'custom', repeatRule: lastFriday });
    const starts = mockOccurrences([{ event: report, skipped: [] }], local(11, 1), local(3, 1) + 365 * 86_400).map((o) =>
      dayKey(new Date(o.start * 1000)),
    );
    expect(starts.slice(0, 3)).toEqual(['2026-11-27', '2026-12-25', '2027-01-29']);

    const fortnightly: RepeatRule = { ...blankRule('weekly'), interval: 2, byDay: ['MO', 'WE'], count: 5 };
    const lesson = ev('l', local(11, 9, 18), local(11, 9, 19), { recurring: true, repeat: 'custom', repeatRule: fortnightly });
    const lessons = mockOccurrences([{ event: lesson, skipped: [] }], local(11, 1), local(12, 31)).map((o) =>
      new Date(o.start * 1000).getDate(),
    );
    expect(lessons).toEqual([9, 11, 23, 25, 7]);
  });
});

describe('the event form', () => {
  it('turns a form into what the backend saves', () => {
    const form = { ...newForm(new Date(2026, 10, 13), 'work:personal', new Date(2026, 10, 13, 10), 15, 600, 90), title: 'Plan' };
    const input = toInput({ ...form, attendees: 'Peter <peter@acme.example>, amina@acme.example', repeat: 'weekdays' });
    expect(input).toMatchObject({
      calendarId: 'work:personal',
      start: local(11, 13, 10),
      end: local(11, 13, 11, 30),
      reminders: [15],
      attendees: ['peter@acme.example', 'amina@acme.example'],
      repeat: 'weekly',
      repeatRule: { freq: 'weekly', byDay: ['MO', 'TU', 'WE', 'TH', 'FR'] },
    });
    expect(toInput({ ...form, attendees: 'not an address' })).toBe('Not a valid address: not an address');
  });

  it('edits one date of a series on its own', () => {
    const series = ev('s', local(11, 9, 9), local(11, 9, 9, 15), { recurring: true, repeat: 'daily', recurrenceId: local(11, 12, 9) });
    const form = formFrom({ ...series, start: local(11, 12, 9), end: local(11, 12, 9, 15) }, [], local(11, 12, 9));
    expect(toInput({ ...form, startTime: '10:00', endTime: '10:15' })).toMatchObject({
      id: 's',
      occurrence: local(11, 12, 9),
      scope: 'this',
      start: local(11, 12, 10),
    });
  });

  it('copies an event as a new one without its people', () => {
    const copy = copiedInput(
      ev('x', local(11, 13, 9), local(11, 13, 10), { attendees: [{ email: 'a@b.example', name: null, status: null }] }),
      local(11, 14, 9),
      local(11, 14, 10),
    );
    expect(copy.id).toBeUndefined();
    expect(copy.attendees).toBeUndefined();
    expect(copy.repeat).toBe('none');
  });
});

describe('contact photos', () => {
  it('scales pictures to fit 256 pixels, never up', () => {
    expect(fitWithin(1024, 512)).toEqual({ width: 256, height: 128 });
    expect(fitWithin(300, 900)).toEqual({ width: 85, height: 256 });
    expect(fitWithin(100, 80)).toEqual({ width: 100, height: 80 });
  });
});

describe('mock backend: 0.8.0', () => {
  it('lists due reminders by its clock, and dismisses and snoozes them', async () => {
    let clock = local(11, 13, 10, 15);
    const api = createMockMailApi({
      reminders: true,
      now: () => clock,
      events: [{ event: ev('soon', local(11, 13, 10, 30), local(11, 13, 11), { reminders: [15] }), skipped: [] }],
      tasks: [],
    });
    const heard: string[][] = [];
    api.on('mail://reminders', ({ reminders }) => heard.push(reminders.map((r) => r.key)));
    const [due] = await api.listDueReminders();
    expect(due).toMatchObject({ kind: 'event', title: 'soon', eventId: 'soon' });
    expect(await api.snoozeReminder(due?.key ?? '', 5)).toEqual([]);
    clock += 6 * 60;
    api.tickReminders();
    expect(heard.at(-1)).toEqual([due?.key]);
    expect(await api.dismissReminder(due?.key ?? '')).toEqual([]);
  });

  it('keeps tasks and contact groups', async () => {
    const api = createMockMailApi();
    const lists = await api.listTaskLists();
    expect(lists.map((l) => l.name)).toEqual(['Tasks', 'Work']);
    const task = await api.saveTask({
      id: null,
      listId: 'device:tasks',
      title: ' Call Peter ',
      notes: '',
      due: null,
      reminder: null,
      importance: 'high',
      completed: false,
      messageId: 'm1',
      categories: [],
    });
    expect(task.title).toBe('Call Peter');
    await api.saveTask({ ...task, completed: true });
    expect((await api.listTasks(null, false)).some((t) => t.id === task.id)).toBe(false);
    expect((await api.listTasks('device:tasks', true)).find((t) => t.id === task.id)?.completedAt).not.toBeNull();
    await expect(api.saveTask({ ...task, title: '' })).rejects.toMatchObject({ kind: 'invalid' });

    const contact = await api.saveContact({ accountId: null, name: 'Peter', emails: ['p@x.example'], phones: [], organization: '', title: '', notes: '' });
    const groups = await api.saveContactGroup({ id: null, accountId: null, name: 'Site team', memberIds: [contact.id, 'gone'] });
    expect(groups[0]?.memberIds).toEqual([contact.id]);
    await expect(api.saveContactGroup({ id: null, accountId: null, name: 'site team', memberIds: [] })).rejects.toMatchObject({
      kind: 'invalid',
    });
    await api.deleteContact(contact.id);
    expect((await api.listContactGroups())[0]?.memberIds).toEqual([]);
  });

  it('reports free/busy and connects Gmail calendars', async () => {
    const api = createMockMailApi({
      freeBusy: { 'peter@acme.example': [{ start: local(11, 13, 11), end: local(11, 13, 12), showAs: 'busy' }] },
      accounts: [
        {
          id: 'g',
          email: 'me@gmail.com',
          displayName: 'Gmail',
          auth: 'google',
          createdAt: 0,
          color: '#2450C8',
          imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
          smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' },
          sizeLimitBytes: null,
          sortOrder: 0,
          syncMonths: null,
        },
      ],
      calendarStatus: [{ accountId: 'g', state: 'unavailable', message: 'Not connected.', connectable: true }],
      oauthConfigured: true,
    });
    const [peter, nobody] = await api.getFreeBusy('g', ['peter@acme.example', 'x@y.example'], local(11, 13), local(11, 14));
    expect(peter?.busy).toHaveLength(1);
    expect(nobody?.unknown).toBe(true);
    await api.connectCalendarAndContacts('g');
    expect((await api.syncCalendars())[0]).toMatchObject({ state: 'synced' });
    expect((await api.listCalendars()).map((c) => c.provider)).toEqual(['google']);
  });
});
