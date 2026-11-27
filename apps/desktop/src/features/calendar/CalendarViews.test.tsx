import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { MailApiProvider } from '../../api/MailApiContext';
import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import { MOCK_CALENDARS, mockEvent, type MockEvent } from '../../api/mock/mockCalendar';
import { createMockMailApi, type MockOptions } from '../../api/mock/mockMailApi';
import type { Account, CalendarEvent } from '../../api/types';
import { FileTasksProvider } from '../files/FileTasks';
import { CalendarView } from './CalendarView';
import { dayText } from './calendarTime';

/** Unix seconds for a local time in November 2026. */
const at = (d: number, h = 0, min = 0) => new Date(2026, 10, d, h, min).getTime() / 1000;
const TODAY = new Date(2026, 10, 13, 10, 15);
const nov = (d: number) => dayText(new Date(2026, 10, d));

function e(id: string, title: string, start: number, end: number, extra: Partial<CalendarEvent> = {}): MockEvent {
  return { event: mockEvent({ id, title, start, end, ...extra }), skipped: [] };
}

const EVENTS: MockEvent[] = [
  e('plan', 'Planning', at(13, 10), at(13, 11)),
  e('a', 'Overlap A', at(13, 13), at(13, 14)),
  e('b', 'Overlap B', at(13, 13, 30), at(13, 14, 30)),
  e('conf', 'Conference', Date.UTC(2026, 10, 13) / 1000, Date.UTC(2026, 10, 14) / 1000, { allDay: true }),
  e('standup', 'Standup', at(9, 9), at(9, 9, 15), { recurring: true, repeat: 'daily' }),
  e('holiday', 'Holiday talk', at(13, 15), at(13, 16), { calendarId: 'work:holidays', isOrganizer: false }),
  e('invited', 'Supplier call', at(12, 11), at(12, 11, 30), {
    isOrganizer: false,
    organizer: { email: 'peter@acme.example', name: 'Peter Mwangi', status: 'accepted' },
    attendees: [{ email: 'you@work.example', name: null, status: 'needs-action' }],
  }),
  e('family', 'Family lunch', at(13, 12), at(13, 12, 45), { calendarId: 'personal:personal' }),
  e('meeting', 'Review with Acme', at(11, 15), at(11, 16), {
    organizer: { email: 'you@work.example', name: null, status: 'accepted' },
    attendees: [{ email: 'peter@acme.example', name: 'Peter', status: 'accepted' }],
  }),
];

interface Setup {
  options?: MockOptions;
  phone?: boolean;
  accounts?: Account[];
}

async function setup({ options = {}, phone = false, accounts = MOCK_ACCOUNTS }: Setup = {}) {
  const api = createMockMailApi({ events: EVENTS, now: () => TODAY.getTime() / 1000, ...options });
  const user = userEvent.setup();
  const onWrite = vi.fn();
  render(
    <MailApiProvider api={api}>
      <FileTasksProvider value={{ run: vi.fn(), busy: false }}>
        <CalendarView accounts={accounts} phone={phone} onClose={vi.fn()} onWrite={onWrite} defaultReminder={15} today={TODAY} />
      </FileTasksProvider>
    </MailApiProvider>,
  );
  await screen.findByRole('region', { name: 'November 2026' });
  await screen.findAllByRole('button', { name: /Planning/ });
  return { api, user, onWrite };
}

async function view(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(within(screen.getByRole('group', { name: 'View' })).getByRole('button', { name }));
}

const grid = () => within(screen.getByRole('grid'));
const cell = (day: number, time: string) => grid().getByRole('gridcell', { name: `${nov(day)}, ${time}` });
const block = (name: RegExp) => screen.getByRole('button', { name });
const pointer = { button: 0, clientX: 0, clientY: 0, pointerId: 1 };

/** Drag from an element to a time-grid cell, like the mouse would. */
function drag(from: HTMLElement, to: HTMLElement, ctrlKey = false) {
  fireEvent.pointerDown(from, pointer);
  fireEvent.pointerMove(to, { ...pointer, ctrlKey });
  fireEvent.pointerUp(to, { ...pointer, ctrlKey });
}

async function saved(api: ReturnType<typeof createMockMailApi>, title: string) {
  let found: CalendarEvent[] = [];
  await waitFor(async () => {
    found = (await api.listEvents(at(1), at(30))).filter((x) => x.title === title);
    expect(found.length).toBeGreaterThan(0);
  });
  return found;
}

describe('calendar views', () => {
  it('shows a day with all-day events on top, overlaps side by side and the time now', async () => {
    const { user } = await setup();
    await view(user, 'Day');
    expect(await screen.findByRole('region', { name: /Friday/ })).toBeInTheDocument();
    const allDay = within(screen.getByRole('group', { name: 'All day' }));
    expect(allDay.getByRole('button', { name: 'Conference' })).toBeInTheDocument();
    const a = block(/^Overlap A,/).closest('.cal-block');
    const b = block(/^Overlap B,/).closest('.cal-block');
    expect(a).toHaveAttribute('data-columns', '2');
    expect(b).toHaveAttribute('data-columns', '2');
    expect(a?.getAttribute('data-column')).not.toBe(b?.getAttribute('data-column'));
    expect(block(/^Planning,/).closest('.cal-block')).toHaveAttribute('data-columns', '1');
    expect(screen.getByRole('img', { name: /^Now, / })).toHaveStyle({ top: `${String((10 * 60 + 15) * 0.8)}px` });
    expect(screen.getByRole('grid', { name: nov(13) })).toBeInTheDocument();
  });

  it('shows a work week, Monday to Friday, and a whole week', async () => {
    const { user } = await setup();
    await view(user, 'Work week');
    const heads = await screen.findAllByRole('button', { name: /^Show / });
    expect(heads.map((h) => h.getAttribute('aria-label'))).toEqual([9, 10, 11, 12, 13].map((d) => `Show ${nov(d)}`));
    expect(screen.getAllByRole('button', { name: /^Standup,/ })).toHaveLength(5);
    await view(user, 'Week');
    expect(screen.getAllByRole('button', { name: /^Show / })).toHaveLength(7);
    // A day's heading opens that day.
    await user.click(screen.getByRole('button', { name: `Show ${nov(12)}` }));
    expect(screen.getByRole('button', { name: 'Day' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('grid', { name: nov(12) })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Next day' }));
    expect(screen.getByRole('grid', { name: nov(13) })).toBeInTheDocument();
  });

  it('jumps to a date from the small month', async () => {
    const { user } = await setup();
    await view(user, 'Day');
    await user.click(screen.getByRole('button', { name: `Go to ${nov(25)}` }));
    expect(screen.getByRole('grid', { name: nov(25) })).toBeInTheDocument();
  });

  it('adds an event by clicking or dragging over empty time', async () => {
    const { user, api } = await setup();
    await view(user, 'Day');
    await user.click(cell(13, '16:00'));
    const quick = within(screen.getByRole('form', { name: 'Quick event' }));
    expect(quick.getByText(`${nov(13)}, 16:00 – 16:30`)).toBeInTheDocument();
    await user.type(quick.getByRole('textbox', { name: 'Title' }), 'Call the bank{Enter}');
    const [bank] = await saved(api, 'Call the bank');
    expect(bank).toMatchObject({ start: at(13, 16), end: at(13, 16, 30), reminders: [15] });

    drag(cell(13, '17:00'), cell(13, '18:00'));
    const second = within(screen.getByRole('form', { name: 'Quick event' }));
    expect(second.getByText(`${nov(13)}, 17:00 – 18:30`)).toBeInTheDocument();
    await user.type(second.getByRole('textbox', { name: 'Title' }), 'Workshop');
    await user.click(second.getByRole('button', { name: 'More options' }));
    const form = within(screen.getByRole('form', { name: 'New event' }));
    expect(form.getByRole('textbox', { name: 'Title' })).toHaveValue('Workshop');
    expect(form.getByLabelText('Start Time')).toHaveValue('17:00');
    expect(form.getByLabelText('End Time')).toHaveValue('18:30');
  });

  it('moves an event by dragging, resizes it by its bottom edge and copies it with Ctrl', async () => {
    const { user, api } = await setup();
    await view(user, 'Work week');
    drag(block(/^Planning,/), cell(13, '14:00'));
    await waitFor(async () => {
      expect((await saved(api, 'Planning'))[0]).toMatchObject({ start: at(13, 14), end: at(13, 15) });
    });
    // To another day.
    drag(await screen.findByRole('button', { name: /^Planning, .*14:00|^Planning, .*02:00/ }), cell(11, '14:00'));
    await waitFor(async () => {
      expect((await saved(api, 'Planning'))[0]).toMatchObject({ start: at(11, 14), end: at(11, 15) });
    });
    const handle = (await screen.findByRole('button', { name: /^Planning,/ })).closest('.cal-block')?.querySelector('.cal-block__resize');
    if (!(handle instanceof HTMLElement)) throw new Error('no resize handle');
    drag(handle, cell(11, '16:30'));
    await waitFor(async () => {
      expect((await saved(api, 'Planning'))[0]).toMatchObject({ start: at(11, 14), end: at(11, 17) });
    });
    drag(screen.getByRole('button', { name: /^Planning,/ }), cell(12, '14:00'), true);
    await waitFor(async () => {
      const copies = await saved(api, 'Planning');
      expect(copies.map((c) => c.start).sort()).toEqual([at(11, 14), at(12, 14)]);
    });
  });

  it('asks before moving one date of a series', async () => {
    const { user, api } = await setup();
    await view(user, 'Day');
    const save = vi.spyOn(api, 'saveEvent');
    drag(block(/^Standup,/), cell(13, '10:30'));
    const ask = within(screen.getByRole('dialog', { name: 'Just this one or the whole series?' }));
    await user.click(ask.getByRole('button', { name: 'Just this one' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(expect.objectContaining({ id: 'standup', occurrence: at(13, 9), scope: 'this', start: at(13, 10, 30) }));
    });
    drag(await screen.findByRole('button', { name: /^Planning,/ }), cell(13, '11:00'));
    await waitFor(() => {
      expect(save).toHaveBeenCalledTimes(2);
    });
    save.mockClear();
    await user.click(screen.getByRole('button', { name: 'Previous day' }));
    drag(await screen.findByRole('button', { name: /^Standup,/ }), cell(12, '08:00'));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'The whole series' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(expect.objectContaining({ id: 'standup', scope: 'all', start: at(9, 8), end: at(9, 8, 15) }));
    });
  });

  it('moves around the grid with the keyboard: arrows, Enter, N and Delete', async () => {
    const { user, api } = await setup();
    await view(user, 'Day');
    const start = cell(13, '09:00');
    expect(start).toHaveAttribute('tabindex', '0');
    start.focus();
    await user.keyboard('{ArrowDown}');
    expect(cell(13, '09:30')).toHaveFocus();
    await user.keyboard('{ArrowRight}');
    await waitFor(() => {
      expect(cell(14, '09:30')).toHaveFocus();
    });
    await user.keyboard('{ArrowLeft}{ArrowUp}');
    await waitFor(() => {
      expect(cell(13, '09:00')).toHaveFocus();
    });
    await user.keyboard('{Enter}');
    expect(screen.getByRole('form', { name: 'Quick event' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('form', { name: 'Quick event' })).not.toBeInTheDocument();

    cell(13, '09:00').focus();
    await user.keyboard('n');
    const form = within(screen.getByRole('form', { name: 'New event' }));
    expect(form.getByLabelText('Start Time')).toHaveValue('09:00');
    await user.click(form.getByRole('button', { name: 'Cancel' }));

    block(/^Planning,/).focus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('article', { name: 'Planning' })).toBeInTheDocument();
    block(/^Planning,/).focus();
    await user.keyboard('{Delete}');
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: /^Planning,/ })).not.toBeInTheDocument();
    });
    expect((await api.listEvents(at(1), at(30))).some((x) => x.id === 'plan')).toBe(false);

    block(/^Standup,/).focus();
    await user.keyboard('{Delete}');
    expect(screen.getByRole('dialog', { name: 'Delete a repeating event' })).toBeInTheDocument();
  });

  it('keeps read-only events read-only, except reminders', async () => {
    const { user, api } = await setup();
    await view(user, 'Day');
    expect(block(/^Holiday talk,/).closest('.cal-block')?.querySelector('.cal-block__resize')).toBeNull();
    expect(block(/^Planning,/).closest('.cal-block')?.querySelector('.cal-block__resize')).not.toBeNull();
    await user.click(block(/^Holiday talk,/));
    const card = within(screen.getByRole('article', { name: 'Holiday talk' }));
    expect(card.getByText(/Kenya holidays is read-only/)).toBeInTheDocument();
    expect(card.queryByRole('button', { name: 'Delete' })).not.toBeInTheDocument();
    await user.click(card.getByRole('button', { name: 'Reminders' }));
    const form = within(screen.getByRole('form', { name: 'Edit event' }));
    expect(form.getByRole('textbox', { name: 'Title' })).toHaveAttribute('readonly');
    expect(form.getByRole('checkbox', { name: 'All day' })).toBeDisabled();
    expect(form.queryByRole('button', { name: 'Find a time' })).not.toBeInTheDocument();
    await user.selectOptions(form.getByRole('combobox', { name: 'Reminder' }), '1 hour before');
    await user.click(form.getByRole('button', { name: 'Save reminders' }));
    await waitFor(async () => {
      expect((await api.getEvent('holiday')).reminders).toEqual([60]);
    });

    // An invitation from someone else: they change it, you answer by email.
    await user.click(screen.getByRole('button', { name: 'Previous day' }));
    await user.click(await screen.findByRole('button', { name: /^Supplier call,/ }));
    const invite = within(screen.getByRole('article', { name: 'Supplier call' }));
    expect(invite.getByText(/Peter Mwangi organises this event/)).toBeInTheDocument();
    await user.click(invite.getByRole('button', { name: 'Reminders' }));
    expect(screen.getByText(/You haven't answered yet · organised by Peter Mwangi/)).toBeInTheDocument();
  });

  it('shows or hides calendars, and puts them side by side in the day view', async () => {
    const { user, api } = await setup();
    await view(user, 'Day');
    const list = within(screen.getByRole('navigation', { name: 'Calendars' }));
    expect(block(/^Family lunch,/)).toBeInTheDocument();
    const hide = vi.spyOn(api, 'setCalendarVisible');
    await user.click(list.getByRole('checkbox', { name: 'Family' }));
    expect(hide).toHaveBeenCalledWith('personal:personal', false);
    expect(screen.queryByRole('button', { name: /^Family lunch,/ })).not.toBeInTheDocument();
    await user.click(list.getByRole('checkbox', { name: 'Family' }));
    expect(await screen.findByRole('button', { name: /^Family lunch,/ })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Side by side' }));
    expect(screen.getByRole('button', { name: 'Side by side' })).toHaveAttribute('aria-pressed', 'true');
    expect(grid().getByRole('gridcell', { name: `Family, ${nov(13)}, 12:00` })).toBeInTheDocument();
    expect(grid().getByRole('gridcell', { name: `Personal Calendar, ${nov(13)}, 12:00` })).toBeInTheDocument();
    // Each calendar's events stay in its own column.
    const familyCol = block(/^Family lunch,/).closest('[data-col-events]');
    const planCol = block(/^Planning,/).closest('[data-col-events]');
    expect(familyCol).not.toBe(planCol);
  });

  it('asks whether to send invitations, but not for Microsoft calendars', async () => {
    const { user, api } = await setup();
    const save = vi.spyOn(api, 'saveEvent');
    await user.click(screen.getByRole('button', { name: 'New event' }));
    const form = within(screen.getByRole('form', { name: 'New event' }));
    await user.type(form.getByRole('textbox', { name: 'Title' }), 'Kick-off');
    await user.type(form.getByRole('combobox', { name: 'Invite people' }), 'peter@acme.example,');
    await user.click(form.getByRole('button', { name: 'Save' }));
    const ask = within(screen.getByRole('dialog', { name: 'Send invitations?' }));
    await user.click(ask.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({ title: 'Kick-off', attendees: ['peter@acme.example'], notifyAttendees: true }),
      );
    });
    expect(await screen.findByRole('article', { name: 'Kick-off' })).toBeInTheDocument();

    // A meeting you organise: deleting it asks about cancellations.
    await user.click(screen.getByRole('button', { name: 'Delete' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Send cancellations?' })).getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(api.cancellations).toHaveLength(1);
    });
  });

  it('lets Microsoft send invitations itself', async () => {
    const { user, api } = await setup({
      options: { calendars: [{ ...MOCK_CALENDARS[0], provider: 'microsoft' } as (typeof MOCK_CALENDARS)[number]] },
    });
    const save = vi.spyOn(api, 'saveEvent');
    await user.click(screen.getByRole('button', { name: 'New event' }));
    const form = within(screen.getByRole('form', { name: 'New event' }));
    await user.type(form.getByRole('textbox', { name: 'Title' }), 'Teams call');
    await user.type(form.getByRole('combobox', { name: 'Invite people' }), 'amina@acme.example,');
    await user.click(form.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(expect.objectContaining({ notifyAttendees: true }));
    });
    expect(screen.queryByRole('dialog', { name: 'Send invitations?' })).not.toBeInTheDocument();
  });

  it('edits one date of a series on its own', async () => {
    const { user, api } = await setup();
    const save = vi.spyOn(api, 'saveEvent');
    await view(user, 'Day');
    await user.click(block(/^Standup,/));
    await user.click(screen.getByRole('button', { name: `Edit only ${nov(13)}` }));
    const form = within(screen.getByRole('form', { name: 'Edit event' }));
    expect(form.getByText(`Changes apply to ${nov(13)} only.`)).toBeInTheDocument();
    expect(form.queryByRole('combobox', { name: 'Repeat' })).not.toBeInTheDocument();
    await user.clear(form.getByRole('textbox', { name: 'Title' }));
    await user.type(form.getByRole('textbox', { name: 'Title' }), 'Standup (long)');
    await user.click(form.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(expect.objectContaining({ occurrence: at(13, 9), scope: 'this', title: 'Standup (long)' }));
    });
    expect((await api.listEvents(at(12), at(15))).map((x) => x.title).filter((t) => t.startsWith('Standup'))).toEqual([
      'Standup',
      'Standup (long)',
      'Standup',
    ]);
  });

  it('sets a custom repeat in the Repeat dialog', async () => {
    const { user, api } = await setup();
    const save = vi.spyOn(api, 'saveEvent');
    await user.click(screen.getByRole('button', { name: 'New event' }));
    const form = within(screen.getByRole('form', { name: 'New event' }));
    await user.type(form.getByRole('textbox', { name: 'Title' }), 'Lessons');
    await user.selectOptions(form.getByRole('combobox', { name: 'Repeat' }), 'Custom…');
    const dialog = within(screen.getByRole('form', { name: 'Custom repeat' }));
    await user.clear(dialog.getByRole('spinbutton', { name: 'Repeat every' }));
    await user.type(dialog.getByRole('spinbutton', { name: 'Repeat every' }), '2');
    // Starts on Friday, so Friday is picked already.
    expect(dialog.getByRole('button', { name: 'Friday' })).toHaveAttribute('aria-pressed', 'true');
    await user.click(dialog.getByRole('button', { name: 'Monday' }));
    await user.click(dialog.getByRole('radio', { name: 'After' }));
    await user.clear(dialog.getByRole('spinbutton', { name: 'Number of times' }));
    await user.type(dialog.getByRole('spinbutton', { name: 'Number of times' }), '6');
    expect(dialog.getByText('Every 2 weeks on Monday and Friday, 6 times')).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Done' }));
    expect(form.getByRole('combobox', { name: 'Repeat' })).toHaveDisplayValue('Every 2 weeks on Monday and Friday, 6 times');
    await user.click(form.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          repeatRule: expect.objectContaining({ freq: 'weekly', interval: 2, byDay: ['FR', 'MO'], count: 6, until: null }) as unknown,
        }),
      );
    });
  });

  it('repeats monthly on "the second Friday" until a date', async () => {
    const { user } = await setup();
    await user.click(screen.getByRole('button', { name: 'New event' }));
    const edit = within(screen.getByRole('form', { name: 'New event' }));
    await user.selectOptions(edit.getByRole('combobox', { name: 'Repeat' }), 'Custom…');
    const monthly = within(screen.getByRole('form', { name: 'Custom repeat' }));
    await user.selectOptions(monthly.getByRole('combobox', { name: 'Unit' }), 'monthly');
    await user.click(monthly.getByRole('radio', { name: 'The second Friday' }));
    await user.click(monthly.getByRole('radio', { name: 'On' }));
    fireEvent.change(monthly.getByLabelText('Last day'), { target: { value: '2027-06-30' } });
    await user.click(monthly.getByRole('button', { name: 'Done' }));
    expect(edit.getByRole('combobox', { name: 'Repeat' })).toHaveDisplayValue(/^Every month on the second Friday, until /);
  });

  it('finds a time when everyone is free', async () => {
    const { user } = await setup({
      options: {
        freeBusy: {
          'peter@acme.example': [{ start: at(13, 11), end: at(13, 12), showAs: 'busy' }],
          'amina@acme.example': [{ start: at(13, 14), end: at(13, 15), showAs: 'tentative' }],
        },
      },
    });
    await user.click(screen.getByRole('button', { name: 'New event' }));
    const form = within(screen.getByRole('form', { name: 'New event' }));
    await user.type(form.getByRole('combobox', { name: 'Invite people' }), 'peter@acme.example, amina@acme.example, ghost@else.example,');
    await user.click(form.getByRole('button', { name: 'Find a time' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Find a time' }));
    const table = within(await dialog.findByRole('table', { name: 'Free and busy times' }));
    const row = (who: RegExp) => table.getByRole('rowheader', { name: who }).closest('tr') as HTMLElement;
    const slot = (who: RegExp, time: string) => row(who).querySelector(`[data-time="${nov(13)}, ${time}"]`);
    expect(slot(/^peter@acme/, '11:00')).toHaveAttribute('data-state', 'busy');
    expect(slot(/^peter@acme/, '12:00')).toHaveAttribute('data-state', 'free');
    expect(slot(/^amina@acme/, '14:30')).toHaveAttribute('data-state', 'tentative');
    expect(slot(/^ghost@else/, '09:00')).toHaveAttribute('data-state', 'unknown');
    // Your own Planning (10:00–11:00) counts as busy.
    expect(slot(/^You/, '10:00')).toHaveAttribute('data-state', 'busy');
    const suggested = within(dialog.getByRole('list', { name: 'Suggested times' }));
    // 11:00 (now is 10:15; the next hour is taken by Peter), 12:00 and 13:00 are free for everyone… but 13:00 is Overlap A for you.
    const first = suggested.getAllByRole('button')[0];
    expect(first).toHaveTextContent(nov(13));
    await user.click(dialog.getByRole('button', { name: `${nov(13)}, 16:00` }));
    expect(screen.queryByRole('dialog', { name: 'Find a time' })).not.toBeInTheDocument();
    expect(form.getByLabelText('Start Time')).toHaveValue('16:00');
    expect(form.getByLabelText('End Time')).toHaveValue('17:00');

    await user.click(form.getByRole('button', { name: 'Find a time' }));
    const again = within(screen.getByRole('dialog', { name: 'Find a time' }));
    await user.click(again.getByRole('button', { name: 'Work week' }));
    const pick = within(await again.findByRole('list', { name: 'Suggested times' })).getAllByRole('button')[0];
    if (!pick) throw new Error('no suggestion');
    const text = pick.textContent;
    await user.click(pick);
    expect(text).toMatch(/–/);
    expect(form.getByLabelText('Start Time')).not.toHaveValue('16:00');
  });

  it('works on a phone: day, month and agenda, a tap opens the full editor, and no dragging', async () => {
    const { user } = await setup({ phone: true });
    const views = within(screen.getByRole('group', { name: 'View' }));
    expect(views.getAllByRole('button').map((b) => b.textContent)).toEqual(['Day', 'Month', 'Agenda']);
    await view(user, 'Day');
    expect(document.querySelector('.cal-block__resize')).toBeNull();
    await user.click(cell(13, '15:30'));
    expect(screen.queryByRole('form', { name: 'Quick event' })).not.toBeInTheDocument();
    const form = within(screen.getByRole('form', { name: 'New event' }));
    expect(form.getByLabelText('Start Time')).toHaveValue('15:30');
    await user.click(form.getByRole('button', { name: 'Cancel' }));
    await user.click(screen.getByRole('button', { name: 'Calendars' }));
    const drawer = within(screen.getByRole('dialog', { name: 'Calendars' }));
    expect(drawer.getByRole('checkbox', { name: 'Family' })).toBeChecked();
    await user.click(drawer.getByRole('button', { name: `Go to ${nov(20)}` }));
    expect(screen.getByRole('grid', { name: nov(20) })).toBeInTheDocument();
  });
});
