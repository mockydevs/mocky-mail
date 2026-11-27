import { act, screen, waitFor, within } from '@testing-library/react';

import { mockEvent } from '../../api/mock/mockCalendar';
import { MOCK_TASKS } from '../../api/mock/mockPlanner';
import type { Task } from '../../api/types';
import { reader, renderApp, rows } from '../../test/renderApp';
import { minutesUntilTomorrow } from './RemindersPanel';

const at = (d: number, h = 0, min = 0) => new Date(2026, 10, d, h, min).getTime() / 1000;

const FIRST_TASK = MOCK_TASKS[0];
if (!FIRST_TASK) throw new Error('sample data missing');
const TASK: Task = { ...FIRST_TASK, id: 't-invoice-due', title: 'Send the invoice', reminder: at(13, 10), due: null };

function start(clock: { now: number }, extra: Parameters<typeof renderApp>[0] = {}) {
  return renderApp({
    reminders: true,
    now: () => clock.now,
    events: [
      { event: mockEvent({ id: 'e-site', title: 'Site visit', start: at(13, 10, 25), end: at(13, 11), location: 'Kilimani', reminders: [15] }), skipped: [] },
    ],
    tasks: [TASK],
    ...extra,
  });
}

const panel = () => within(screen.getByRole('dialog', { name: /reminders?$/i }));

describe('reminders', () => {
  it('lists what is due, and dismisses and snoozes it', async () => {
    const clock = { now: at(13, 10, 15) };
    const { user, api } = start(clock);
    await rows();
    const list = within(await screen.findByRole('list', { name: 'Due reminders' }));
    expect(panel().getByRole('heading', { name: '2 reminders' })).toBeInTheDocument();
    const visit = within(list.getByRole('listitem', { name: 'Event: Site visit' }));
    expect(visit.getByText(/Starts in 10 minutes/)).toBeInTheDocument();
    expect(visit.getByText(/Kilimani/)).toBeInTheDocument();
    expect(list.getByRole('listitem', { name: 'Task: Send the invoice' })).toBeInTheDocument();

    await user.click(visit.getByRole('button', { name: 'Snooze Site visit' }));
    const snooze = vi.spyOn(api, 'snoozeReminder');
    await user.click(screen.getByRole('menuitem', { name: '5 minutes' }));
    expect(snooze).toHaveBeenCalledWith('event:e-site:' + String(at(13, 10, 25)), 5);
    await waitFor(() => {
      expect(screen.queryByRole('listitem', { name: 'Event: Site visit' })).not.toBeInTheDocument();
    });

    await user.click(panel().getByRole('button', { name: 'Dismiss Send the invoice' }));
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: /reminders?$/i })).not.toBeInTheDocument();
    });

    // Five minutes later the snoozed one comes back.
    clock.now += 6 * 60;
    act(() => {
      api.tickReminders();
    });
    expect(await screen.findByRole('listitem', { name: 'Event: Site visit' })).toBeInTheDocument();
    await user.click(panel().getByRole('button', { name: 'Dismiss all' }));
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: /reminders?$/i })).not.toBeInTheDocument();
    });
    expect(await api.listDueReminders()).toEqual([]);
  });

  it('opens an event in the calendar, and a task in Tasks', async () => {
    const clock = { now: at(13, 10, 15) };
    const { user } = start(clock);
    await rows();
    await user.click(await screen.findByRole('button', { name: 'Open Site visit' }));
    const card = await screen.findByRole('article', { name: 'Site visit' });
    expect(within(card).getByText('Kilimani')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Open Send the invoice' }));
    const form = within(await screen.findByRole('form', { name: 'Task' }));
    expect(form.getByRole('textbox', { name: 'Title' })).toHaveValue('Send the invoice');
  });

  it('opens a flagged message, and closes until something new is due', async () => {
    const clock = { now: at(16, 17, 30) };
    const { user, api } = start(clock, { events: [], tasks: [] });
    await rows();
    expect(await screen.findByRole('listitem', { name: 'Flagged message: Logo files, final round' })).toBeInTheDocument();
    await user.click(panel().getByRole('button', { name: 'Close reminders' }));
    expect(screen.queryByRole('dialog', { name: /reminders?$/i })).not.toBeInTheDocument();
    act(() => {
      api.tickReminders();
    });
    // The same list doesn't bring it back.
    expect(screen.queryByRole('dialog', { name: /reminders?$/i })).not.toBeInTheDocument();
    await api.setFlags(['m2'], { flagged: true, flagDue: at(16, 9) });
    act(() => {
      api.tickReminders();
    });
    const list = within(await screen.findByRole('list', { name: 'Due reminders' }));
    await user.click(list.getByRole('button', { name: 'Open Logo files, final round' }));
    await waitFor(() => {
      expect(within(reader()).getByRole('heading', { name: 'Logo files, final round' })).toBeInTheDocument();
    });
  });

  it('fills the screen on a phone', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query.startsWith('(max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    try {
      const clock = { now: at(13, 10, 15) };
      start(clock);
      await rows();
      const sheet = await screen.findByRole('dialog', { name: /reminders?$/i });
      expect(sheet).toHaveAttribute('aria-modal', 'true');
      expect(sheet).toHaveClass('reminders--sheet');
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('snoozes until tomorrow morning', () => {
    expect(minutesUntilTomorrow(at(13, 21))).toBe(12 * 60);
  });
});
