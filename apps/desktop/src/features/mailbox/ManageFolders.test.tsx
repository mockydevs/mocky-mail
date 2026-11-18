import { screen, waitFor, within } from '@testing-library/react';

import { accountToggle, renderApp, rows } from '../../test/renderApp';

async function manage() {
  const ctx = renderApp();
  await rows();
  // Work is open on start; its folder list ends with "Manage folders".
  const nav = within(screen.getByRole('navigation', { name: 'Mailboxes' }));
  if (accountToggle('Work').getAttribute('aria-expanded') !== 'true') await ctx.user.click(accountToggle('Work'));
  await ctx.user.click(nav.getByRole('button', { name: 'Manage folders' }));
  const dialog = within(screen.getByRole('dialog', { name: 'Manage folders' }));
  return { ...ctx, dialog };
}

describe('manage folders', () => {
  it('creates a folder inside another, and it appears in the sidebar', async () => {
    const { user, dialog } = await manage();
    await user.type(dialog.getByRole('textbox', { name: 'Name' }), 'Wateja 2026');
    await user.selectOptions(dialog.getByRole('combobox', { name: 'Put it in' }), 'Clients');
    await user.click(dialog.getByRole('button', { name: 'Create folder' }));
    expect(await dialog.findByText('Folder "Wateja 2026" created.')).toBeInTheDocument();
    const yours = within(dialog.getByRole('list', { name: 'Your folders' }));
    expect(yours.getByText(/^Clients \/ Wateja 2026/)).toBeInTheDocument();
    await waitFor(() => {
      expect(within(screen.getByRole('navigation', { name: 'Mailboxes' })).getAllByText('Wateja 2026').length).toBeGreaterThan(0);
    });
  });

  it('renames a folder and refuses a bad name with the server’s reason', async () => {
    const { user, dialog } = await manage();
    await user.click(dialog.getByRole('button', { name: 'Rename Clients' }));
    const input = dialog.getByRole('textbox', { name: 'New name for Clients' });
    await user.clear(input);
    await user.type(input, 'Customers');
    await user.click(dialog.getByRole('button', { name: 'Save' }));
    expect(await dialog.findByText('Renamed to "Customers".')).toBeInTheDocument();

    await user.type(dialog.getByRole('textbox', { name: 'Name' }), 'a/b');
    await user.click(dialog.getByRole('button', { name: 'Create folder' }));
    expect(await dialog.findByRole('alert')).toHaveTextContent('can\'t contain "/"');
  });

  it('deletes only after confirming, and says the mail goes too', async () => {
    const { user, dialog } = await manage();
    const label = 'Clients / Acme';
    await user.click(dialog.getByRole('button', { name: `Delete ${label}` }));
    const confirm = within(screen.getByRole('dialog', { name: /^Delete / }));
    expect(confirm.getByText(/deleted from the server\. This can't be undone\./)).toBeInTheDocument();
    await user.click(confirm.getByRole('button', { name: 'Delete folder' }));
    await waitFor(() => {
      expect(dialog.queryByRole('button', { name: `Delete ${label}` })).not.toBeInTheDocument();
    });
  });

  it('refuses to delete a folder that still has folders inside', async () => {
    const { user, dialog } = await manage();
    await user.click(dialog.getByRole('button', { name: 'Delete Clients' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Delete Clients?' })).getByRole('button', { name: 'Delete folder' }));
    expect(await dialog.findByRole('alert')).toHaveTextContent('has folders inside it');
  });

  it('does not offer built-in folders', async () => {
    const { dialog } = await manage();
    for (const builtIn of ['Inbox', 'Sent', 'Drafts', 'Trash']) {
      expect(dialog.queryByRole('button', { name: `Delete ${builtIn}` })).not.toBeInTheDocument();
    }
  });
});
