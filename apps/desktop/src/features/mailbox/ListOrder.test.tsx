import { act, cleanup, fireEvent, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { at, MOCK_NOW } from '../../api/mock/mockData';
import type { MessageListItem } from '../../api/types';
import { accountFolders, reader, renderApp, rows, unified } from '../../test/renderApp';
import { dueFor, dueFromDate, dueLabel, isOverdue } from './flagDue';
import { groupRows } from './grouping';
import { ORDER_STORAGE_KEY, readOrder } from './useListView';

const ids = async () => (await rows()).getAllByRole('button').map((b) => b.dataset['id']);
const headings = () => [...document.querySelectorAll('.rows__group')].map((h) => h.textContent);

describe('sorting and grouping the list', () => {
  it('sorts by subject A to Z, then Z to A, and remembers it', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: /^Sort and group/ }));
    const menu = within(screen.getByRole('menu', { name: /^Sort and group/ }));
    const sortBy = within(menu.getByRole('group', { name: 'Sort by' }));
    expect(sortBy.getByRole('menuitemradio', { name: 'Date' })).toHaveAttribute('aria-checked', 'true');
    expect(within(menu.getByRole('group', { name: 'Group by' })).getByRole('menuitemradio', { name: 'Date' })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await user.click(sortBy.getByRole('menuitemradio', { name: 'Subject' }));
    // Still under day headings, A to Z inside each day.
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m5', 'm6', 'm3', 'm2', 'm1', 'm7']);
    });
    await user.click(screen.getByRole('button', { name: /^Sort and group: Subject, A to Z/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Importance' }));
    expect(await ids()).toEqual(['m1', 'm5', 'm6', 'm3', 'm2', 'm7']);
    await user.click(screen.getByRole('button', { name: /^Sort and group: Subject, A to Z/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Z to A' }));
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m1', 'm7', 'm2', 'm3', 'm6', 'm5']);
    });
    expect(JSON.parse(window.localStorage.getItem(ORDER_STORAGE_KEY) ?? '{}')).toEqual({
      sort: 'subject',
      descending: true,
      groupBy: 'importance',
    });

    cleanup();
    renderApp();
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m1', 'm7', 'm2', 'm3', 'm6', 'm5']);
    });
  });

  it('sorts by size, largest first', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: /^Sort and group/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Size' }));
    await vi.waitFor(async () => {
      expect((await ids())[0]).toBe('m7');
    });
    expect(screen.getByRole('button', { name: 'Sort and group: Size, Largest first' })).toBeInTheDocument();
  });

  it('groups by importance and by sender, client-side', async () => {
    const { user } = renderApp();
    await rows();
    expect(headings()).toEqual(['Today', 'Yesterday', 'Wednesday', 'Tuesday']);
    await user.click(screen.getByRole('button', { name: /^Sort and group/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Importance' }));
    expect(headings()).toEqual(['High importance', 'Normal importance']);
    expect(await ids()).toEqual(['m1', 'm2', 'm3', 'm5', 'm6', 'm7']);
    await user.click(screen.getByRole('button', { name: /^Sort and group/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Sender' }));
    expect(headings()).toEqual(['Wanjiru Kamau', 'Build server', 'Brian Otieno', 'Calendar', 'Peter Mwangi', 'Grace Njeri']);
  });

  it('keeps pinned messages at the top of a folder under their own heading', async () => {
    const { user, api } = renderApp();
    await rows();
    await api.setFlags(['m7'], { pinned: true });
    act(() => {
      api.emit('mail://changed', { accountId: 'personal', folderIds: [] });
    });
    await vi.waitFor(async () => {
      expect((await ids())[0]).toBe('m7');
    });
    expect(headings()[0]).toBe('Pinned');
    expect((await rows()).getByRole('button', { name: /Grace Njeri/ })).toHaveAccessibleName(/Pinned/);
    // Unpin from the row's quick actions.
    await user.hover((await rows()).getByRole('button', { name: /Grace Njeri/ }));
    const actions = within(screen.getByRole('toolbar', { name: 'Actions for Photos from the weekend' }));
    await user.click(actions.getByRole('button', { name: 'Unpin' }));
    await vi.waitFor(async () => {
      expect((await ids())[0]).toBe('m1');
    });
  });
});

describe('filters', () => {
  it('shows messages with attachments, to me, or pinned from the More menu', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'More filters' }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Has attachments' }));
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m1', 'm6', 'm7']);
    });
    expect(screen.getByRole('button', { name: 'More filters: Has attachments' })).toHaveTextContent('Has attachments');
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute('aria-pressed', 'false');

    await user.click(screen.getByRole('button', { name: /^More filters/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'To me' }));
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m1', 'm2', 'm3', 'm5', 'm6', 'm7']);
    });

    await api.setFlags(['m5'], { pinned: true });
    await user.click(screen.getByRole('button', { name: /^More filters/ }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Pinned' }));
    await vi.waitFor(async () => {
      expect(await ids()).toEqual(['m5']);
    });
    await user.click(screen.getByRole('button', { name: 'All' }));
    await vi.waitFor(async () => {
      expect(await ids()).toHaveLength(6);
    });
  });
});

describe('what a row shows', () => {
  it('shows importance, categories with their colours and a due date', async () => {
    const { user } = renderApp();
    const list = await rows();
    const wanjiru = list.getByRole('button', { name: /Wanjiru Kamau/ });
    expect(within(wanjiru).getByLabelText('High importance')).toHaveTextContent('!');
    const chip = within(wanjiru).getByText('Clients');
    expect(chip).toHaveClass('chip');
    expect(chip.getAttribute('style')).toContain('#2450C8');
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Clients/ }));
    const amina = (await rows('Clients')).getByRole('button', { name: /Amina Hassan/ });
    // Due Mon 16 Nov, three days after the sample "now".
    expect(within(amina).getByText('Due Mon 16 Nov')).not.toHaveClass('row__due--overdue');
  });

  it('marks a message read or unread from the left edge of its row', async () => {
    renderApp();
    const list = await rows();
    const edge = list.getByRole('button', { name: /Build server/ }).querySelector('.row__edge');
    if (!edge) throw new Error('no edge');
    fireEvent.click(edge);
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
    // It did not open the message.
    expect(within(reader()).queryByRole('heading', { name: 'Nightly build passed on main' })).not.toBeInTheDocument();
    fireEvent.click(edge);
    expect(await unified().findByRole('button', { name: 'Inbox 4 unread' })).toBeInTheDocument();
  });
});

describe('due dates', () => {
  // Fri 13 Nov 2026, 10:15 in Nairobi (07:15 UTC); the tests use local time.
  it('offers today, tomorrow, Friday and next Monday at the end of the working day', () => {
    const local = (iso: string) => Math.floor(new Date(iso).getTime() / 1000);
    const friday = local('2026-11-13T10:15:00');
    expect(dueFor('today', friday)).toBe(local('2026-11-13T17:00:00'));
    expect(dueFor('tomorrow', friday)).toBe(local('2026-11-14T17:00:00'));
    expect(dueFor('thisWeek', friday)).toBe(local('2026-11-13T17:00:00'));
    expect(dueFor('nextWeek', friday)).toBe(local('2026-11-16T17:00:00'));
    const tuesday = local('2026-11-10T09:00:00');
    expect(dueFor('thisWeek', tuesday)).toBe(local('2026-11-13T17:00:00'));
    expect(dueFor('nextWeek', tuesday)).toBe(local('2026-11-16T17:00:00'));
    // After 17:00, "today" means tonight.
    expect(dueFor('today', local('2026-11-13T18:30:00'))).toBe(local('2026-11-13T23:59:00'));
    expect(dueFromDate('2026-12-01')).toBe(local('2026-12-01T17:00:00'));
    expect(dueFromDate('nope')).toBeNull();
  });

  it('reads as a day and says when it is overdue', () => {
    const now = Math.floor(new Date('2026-11-13T10:00:00').getTime() / 1000);
    expect(dueLabel(now + 3600, now)).toBe('Due today');
    expect(dueLabel(now + 86_400, now)).toBe('Due tomorrow');
    expect(dueLabel(now - 86_400, now)).toBe('Overdue: due yesterday');
    expect(isOverdue(now - 1, now)).toBe(true);
  });
});

describe('groupRows and readOrder', () => {
  const item = (id: string, extra: Partial<MessageListItem>): MessageListItem => ({
    id, accountId: 'a', folderId: 'f', from: null, to: [], subject: id, preview: '', date: MOCK_NOW, unread: false,
    flagged: false, answered: false, hasAttachments: false, pinned: false, importance: 'normal', categories: [],
    flagDue: null, size: 0, ...extra,
  });

  it('gathers groups in order of first appearance, keeping the sort inside', () => {
    const rowsIn = [
      item('a', { date: at('2026-11-13T06:00:00Z') }),
      item('b', { date: at('2026-11-01T06:00:00Z') }),
      item('c', { date: at('2026-11-13T05:00:00Z') }),
    ];
    expect(groupRows(rowsIn, 'date', MOCK_NOW).map((g) => [g.label, g.items.map((m) => m.id)])).toEqual([
      ['Today', ['a', 'c']],
      ['November', ['b']],
    ]);
  });

  it('falls back to the default order when storage is empty or corrupt', () => {
    expect(readOrder(null)).toEqual({ sort: 'date', descending: true, groupBy: 'date' });
    expect(readOrder({ getItem: () => '{"sort":"nope","descending":"x"}' })).toEqual({ sort: 'date', descending: true, groupBy: 'date' });
    expect(readOrder({ getItem: () => '{' })).toEqual({ sort: 'date', descending: true, groupBy: 'date' });
  });
});
