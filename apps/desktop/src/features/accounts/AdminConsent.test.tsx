import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import { MOCK_ADMIN_CONSENT_URL } from '../../api/mock/mockMailApi';
import type { Account } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

const panelTitle = 'Your organisation needs an administrator to approve Mocky Mail.';
const panelOf = () => within(screen.getByRole('alert', { name: panelTitle }));

const [WORK] = MOCK_ACCOUNTS;
if (!WORK) throw new Error('sample data missing');
const OUTLOOK: Account = { ...WORK, id: 'office', email: 'me@contoso.example', displayName: 'Office', auth: 'microsoft', sortOrder: 1 };

describe('an administrator must approve Mocky Mail', () => {
  it('adding a Microsoft account: opens the approval page, copies a message, and tries again', async () => {
    const { user, api } = renderApp({ accounts: [], oauthConfigured: true, adminConsent: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const form = within(screen.getByRole('form', { name: 'Add an email account' }));
    await user.click(form.getByRole('button', { name: 'Continue with Microsoft' }));
    const panel = await within(screen.getByRole('form', { name: 'Add an email account' })).findByRole('alert', {
      name: panelTitle,
    });
    expect(panel).toBeInTheDocument();

    await user.click(panelOf().getByRole('button', { name: 'Open the approval page' }));
    expect(api.opened).toEqual([MOCK_ADMIN_CONSENT_URL]);

    await user.click(panelOf().getByRole('button', { name: 'Copy a message for your administrator' }));
    expect(await panelOf().findByRole('status')).toHaveTextContent('Message copied');
    const copied = await navigator.clipboard.readText();
    expect(copied).toContain(MOCK_ADMIN_CONSENT_URL);
    expect(copied).toMatch(/approve Mocky Mail for our organisation/);

    // Still waiting for the administrator: the panel stays.
    const add = vi.spyOn(api, 'addOAuthAccount');
    await user.click(panelOf().getByRole('button', { name: 'Try again' }));
    expect(add).toHaveBeenLastCalledWith(expect.objectContaining({ provider: 'microsoft' }));
    expect(await screen.findByRole('alert', { name: panelTitle })).toBeInTheDocument();

    api.approveAdminConsent();
    await user.click(panelOf().getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('list', { name: 'All accounts' })).toBeInTheDocument();
    expect(add).toHaveBeenCalledTimes(2);
  });

  it('signing in again from Settings › Accounts', async () => {
    const { user, api } = renderApp({ accounts: [WORK, OUTLOOK], oauthConfigured: true, adminConsent: true });
    await rows();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const region = within(screen.getByRole('region', { name: 'Settings' }));
    const row = within(region.getByText('me@contoso.example').closest('li') as HTMLElement);
    await user.click(row.getByRole('button', { name: 'Sign in again' }));
    const panel = within(await row.findByRole('alert', { name: panelTitle }));
    // The plain error isn't shown as well.
    expect(row.queryByText('Sign-in was not accepted')).not.toBeInTheDocument();
    await user.click(panel.getByRole('button', { name: 'Open the approval page' }));
    expect(api.opened).toEqual([MOCK_ADMIN_CONSENT_URL]);

    api.approveAdminConsent();
    await user.click(panel.getByRole('button', { name: 'Try again' }));
    expect(await row.findByText('Signed in again.')).toBeInTheDocument();
    expect(row.queryByRole('alert', { name: panelTitle })).not.toBeInTheDocument();
  });

  it('connecting a Microsoft calendar', async () => {
    const { user, api } = renderApp({
      accounts: [WORK, OUTLOOK],
      oauthConfigured: true,
      adminConsent: true,
      calendarStatus: [
        { accountId: 'work', state: 'synced', message: null },
        { accountId: 'office', state: 'unavailable', message: 'Not connected yet.', connectable: true },
      ],
    });
    await rows();
    await user.click(screen.getByRole('button', { name: 'Calendar' }));
    const card = within(await screen.findByRole('region', { name: 'Connect me@contoso.example' }));
    await user.click(card.getByRole('button', { name: 'Connect' }));
    const panel = within(await card.findByRole('alert', { name: panelTitle }));
    await user.click(panel.getByRole('button', { name: 'Copy a message for your administrator' }));
    expect(await navigator.clipboard.readText()).toContain(MOCK_ADMIN_CONSENT_URL);

    api.approveAdminConsent();
    await user.click(panel.getByRole('button', { name: 'Try again' }));
    expect(
      await card.findByRole('heading', { name: 'Microsoft calendar, contacts and tasks connected' }),
    ).toBeInTheDocument();
  });

  it('shows the message to copy by hand when the clipboard refuses', async () => {
    const { user } = renderApp({ accounts: [], oauthConfigured: true, adminConsent: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.click(screen.getByRole('button', { name: 'Continue with Microsoft' }));
    await screen.findByRole('alert', { name: panelTitle });
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValueOnce(new Error('denied'));
    await user.click(panelOf().getByRole('button', { name: 'Copy a message for your administrator' }));
    const text = await panelOf().findByRole('textbox');
    expect(text).toHaveAttribute('readonly');
    expect((text as HTMLTextAreaElement).value).toContain(MOCK_ADMIN_CONSENT_URL);
  });
});
