import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import { accountFolders, reader, renderApp, rows, accountToggle, queryAccountToggle } from '../../test/renderApp';

async function openSettings(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  return { ...ctx, region: within(screen.getByRole('region', { name: 'Settings' })) };
}

describe('settings', () => {
  it('changes the theme and saves it to the backend', async () => {
    const { user, api, region } = await openSettings();
    await user.click(region.getByRole('radio', { name: 'Dark' }));
    expect(document.documentElement.dataset['theme']).toBe('dark');
    expect((await api.getSettings()).theme).toBe('dark');
    await user.click(region.getByRole('radio', { name: 'Match my system' }));
    expect(document.documentElement.dataset['theme']).toBe('light');
  });

  it('sets how often to check for mail, and the read delay when it applies', async () => {
    const { user, api, region } = await openSettings();
    await user.selectOptions(region.getByRole('combobox', { name: 'Check for new mail every' }), '5 minutes');
    expect((await api.getSettings()).checkEveryMins).toBe(5);
    expect(region.queryByRole('combobox', { name: 'Mark as read after' })).not.toBeInTheDocument();
    await user.click(region.getByRole('radio', { name: /after a few seconds/ }));
    await user.selectOptions(region.getByRole('combobox', { name: 'Mark as read after' }), '10 seconds');
    expect((await api.getSettings()).markReadDelaySecs).toBe(10);
  });

  it('rolls back a setting the backend refuses', async () => {
    const { user, api, region } = await openSettings();
    api.failNext('setSettings', { kind: 'storage', message: 'Disk full.' });
    await user.click(region.getByRole('radio', { name: 'Always load images' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not save settings');
    expect(region.getByRole('radio', { name: /Ask before loading images/ })).toBeChecked();
  });

  it('loads remote images in every message when allowed', async () => {
    const { user, region } = await openSettings();
    await user.click(region.getByRole('radio', { name: 'Always load images' }));
    await user.click(region.getByRole('button', { name: 'Close settings' }));
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const frame = await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(frame.getAttribute('srcdoc')).toContain('https://ci.example/badge.svg');
    expect(within(reader()).queryByRole('button', { name: /Show images/ })).not.toBeInTheDocument();
  });

  it('turns desktop notifications on and off in the backend settings', async () => {
    const { user, api, region } = await openSettings();
    const box = region.getByRole('checkbox', { name: /Show a notification/ });
    expect(box).toBeChecked();
    await user.click(box);
    expect(box).not.toBeChecked();
    expect((await api.getSettings()).notifications).toBe(false);
  });

  it('removes an account only after confirming', async () => {
    const { user, region } = await openSettings();
    const row = within(region.getByText('you@personal.example').closest('li') as HTMLElement);
    await user.click(row.getByRole('button', { name: 'Remove' }));
    const dialog = screen.getByRole('dialog', { name: 'Remove you@personal.example?' });
    expect(dialog).toHaveTextContent('Your mail on the server is not touched');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(accountToggle('Personal')).toBeInTheDocument();

    await user.click(row.getByRole('button', { name: 'Remove' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove account' }));
    await vi.waitFor(() => {
      expect(queryAccountToggle('Personal')).not.toBeInTheDocument();
    });
    expect(accountFolders('Work')).toBeDefined();
  });

  it('goes back to the inbox when the account whose folder was open is removed', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Clients/ }));
    await rows('Clients');
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const region = within(screen.getByRole('region', { name: 'Settings' }));
    const row = within(region.getByText('you@work.example').closest('li') as HTMLElement);
    await user.click(row.getByRole('button', { name: 'Remove' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove account' }));
    await vi.waitFor(() => {
      expect(queryAccountToggle('Work')).not.toBeInTheDocument();
    });
    await user.click(region.getByRole('button', { name: 'Close settings' }));

    // Not the removed account's folder, and a new message is from an account that exists.
    const inbox = await rows('Inbox');
    expect(await inbox.findByRole('button', { name: /Brian Otieno/ })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const from = within(screen.getByRole('form', { name: 'New message' })).getByLabelText('From');
    expect(from).toHaveValue('personal');
  });

  it('signs a Google or Microsoft account in again, and explains a failure', async () => {
    const [work, personal] = MOCK_ACCOUNTS;
    if (!work || !personal) throw new Error('mock accounts missing');
    const { user, api, region } = await openSettings({ accounts: [work, { ...personal, auth: 'google' }] });
    const reconnect = vi.spyOn(api, 'reconnectAccount');
    // Only accounts that sign in with a provider have it.
    const password = within(region.getByText('you@work.example').closest('li') as HTMLElement);
    expect(password.queryByRole('button', { name: 'Sign in again' })).not.toBeInTheDocument();

    const row = within(region.getByText('you@personal.example').closest('li') as HTMLElement);
    await user.click(row.getByRole('button', { name: 'Sign in again' }));
    expect(await row.findByText('Signed in again.')).toBeInTheDocument();
    expect(reconnect).toHaveBeenCalledWith('personal');

    api.failNext('reconnectAccount', { kind: 'auth', message: 'You signed in as someone-else@gmail.com.' });
    await user.click(row.getByRole('button', { name: 'Sign in again' }));
    expect(await row.findByRole('alert')).toBeInTheDocument();
    expect(row.queryByText('Signed in again.')).not.toBeInTheDocument();
    // Ready to try again.
    expect(row.getByRole('button', { name: 'Sign in again' })).toBeEnabled();
  });

  it('updates a password without keeping it, and explains a rejection', async () => {
    const { user, api, region } = await openSettings();
    const spy = vi.spyOn(api, 'updateAccount');
    const row = within(region.getByText('you@work.example').closest('li') as HTMLElement);
    await user.click(row.getByRole('button', { name: 'Change password' }));
    await user.type(row.getByLabelText('New password or app password'), 'wrong');
    await user.click(row.getByRole('button', { name: 'Update password' }));
    expect(await row.findByRole('alert')).toHaveTextContent('Sign-in was not accepted');
    expect(spy).toHaveBeenCalledWith('work', { password: 'wrong' });

    await user.click(row.getByRole('button', { name: 'Change password' }));
    const field = row.getByLabelText('New password or app password');
    await user.type(field, 'n3w-secret');
    await user.click(row.getByRole('button', { name: 'Update password' }));
    expect(await row.findByText('Password updated.')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain('n3w-secret');
  });

  it('renames an account', async () => {
    const { user, region } = await openSettings();
    const row = within(region.getByText('you@work.example').closest('li') as HTMLElement);
    const name = row.getByLabelText('Name shown to recipients');
    await user.clear(name);
    await user.type(name, 'Mocky Digital');
    await user.click(row.getByRole('button', { name: 'Save name' }));
    expect(await screen.findByRole('list', { name: 'Mocky Digital' })).toBeInTheDocument();
  });
});
