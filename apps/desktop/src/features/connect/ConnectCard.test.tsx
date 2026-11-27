import { screen, waitFor, within } from '@testing-library/react';

import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import type { Account } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

const BASE = MOCK_ACCOUNTS[0];
if (!BASE) throw new Error('sample data missing');

const GMAIL: Account = {
  ...BASE,
  id: 'gmail',
  email: 'me@gmail.com',
  displayName: 'Gmail',
  auth: 'google',
  sortOrder: 2,
};
const OUTLOOK: Account = {
  ...BASE,
  id: 'outlook',
  email: 'me@outlook.com',
  displayName: 'Outlook',
  auth: 'microsoft',
  sortOrder: 3,
};

const options = (oauthConfigured: boolean) => ({
  accounts: [...MOCK_ACCOUNTS, GMAIL, OUTLOOK],
  oauthConfigured,
  calendarStatus: [
    { accountId: 'work', state: 'synced' as const, message: null },
    { accountId: 'gmail', state: 'unavailable' as const, message: 'Not connected yet.', connectable: true },
  ],
  books: [{ accountId: 'gmail', state: 'unavailable' as const, message: 'Not connected yet.', connectable: true }],
});

describe('connecting Gmail and Microsoft calendars, contacts and tasks', () => {
  it('connects from the calendar, and keeps the card with the reason when it fails', async () => {
    const { user, api } = renderApp(options(false));
    await rows();
    await user.click(screen.getByRole('button', { name: 'Calendar' }));
    const card = within(await screen.findByRole('region', { name: 'Connect me@gmail.com' }));
    expect(card.getByRole('heading', { name: 'Connect your Google calendar, contacts and tasks' })).toBeInTheDocument();
    await user.click(card.getByRole('button', { name: 'Connect' }));
    expect(await card.findByRole('alert')).toHaveTextContent('Google calendar and contacts access is not set up in this build yet.');
    expect(card.getByRole('button', { name: 'Try again' })).toBeEnabled();
    // Not shown as a sync problem as well.
    expect(screen.queryByRole('list', { name: 'Calendar sync' })).not.toBeInTheDocument();
    const connect = vi.spyOn(api, 'connectCalendarAndContacts');
    await user.click(card.getByRole('button', { name: 'Try again' }));
    expect(connect).toHaveBeenCalledWith('gmail');
  });

  it('says when it is connected, and shows the card in Contacts, Tasks and automatic replies', async () => {
    const { user, api } = renderApp(options(true));
    await rows();
    await user.click(screen.getByRole('button', { name: 'Contacts' }));
    expect(await screen.findByRole('region', { name: 'Connect me@gmail.com' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close contacts' }));

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const replies = within(await screen.findByRole('region', { name: 'Automatic replies' }));
    expect(await replies.findByRole('region', { name: 'Connect me@gmail.com' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close settings' }));

    await user.click(screen.getByRole('button', { name: 'Tasks' }));
    const card = within(await screen.findByRole('region', { name: 'Connect me@gmail.com' }));
    await user.click(card.getByRole('button', { name: 'Connect' }));
    expect(await card.findByRole('heading', { name: 'Google calendar, contacts and tasks connected' })).toBeInTheDocument();
    await waitFor(async () => {
      expect((await api.listTaskLists()).some((l) => l.accountId === 'gmail')).toBe(true);
    });
    expect(await screen.findByRole('button', { name: 'My Tasks' })).toBeInTheDocument();
  });
});
