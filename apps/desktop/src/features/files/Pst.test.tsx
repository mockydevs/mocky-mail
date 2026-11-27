import { configure, screen, waitFor, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_SETTINGS } from '../../api/mock/mockData';
import { renderApp, rows } from '../../test/renderApp';

// Other work on this machine can slow the app down; give each step time.
configure({ asyncUtilTimeout: 4000 });

async function openSection(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  const section = within(screen.getByRole('region', { name: 'Import and export' }));
  return { ...ctx, section };
}

describe('Outlook .pst import', () => {
  it('explains where things go, imports into the chosen account and sums up by kind', async () => {
    const { user, api, section } = await openSection();
    expect(
      section.getByText(/Mail goes to a new folder, Imported from Outlook, on that account; contacts and calendar go to the account's address book and calendar/),
    ).toBeInTheDocument();
    const spy = vi.spyOn(api, 'importPst');
    await user.selectOptions(section.getByRole('combobox', { name: 'Import into account' }), 'Personal (you@personal.example)');
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    expect(spy).toHaveBeenCalledWith('personal');

    const summary = within(await screen.findByRole('dialog', { name: 'Outlook import finished' }));
    expect(summary.getByRole('status')).toHaveTextContent(
      '4 messages, 1 contact, 1 calendar event, 2 tasks and 1 note imported.',
    );
    const kinds = within(summary.getByRole('list', { name: 'Imported by kind' }));
    expect(kinds.getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'Messages4',
      'Contacts1',
      'Calendar events1',
      'Tasks2',
      'Notes1',
    ]);
    await user.click(summary.getByText('Why one item was left out'));
    expect(summary.getByText(/“Scanned drawings” was left out/)).toBeVisible();

    await user.click(summary.getByRole('button', { name: 'Open folder' }));
    expect(await screen.findByRole('heading', { level: 1, name: 'Imported from Outlook' })).toBeInTheDocument();
    const contacts = await api.listContacts('Njoroge');
    expect(contacts.map((c) => c.accountId)).toEqual(['personal']);
  });

  it('keeps going when the progress dialog is closed, and a toast says when it is done', async () => {
    const { user, api, section } = await openSection();
    const release = api.holdFileTasks();
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    const progress = within(await screen.findByRole('dialog', { name: 'Importing from Outlook into you@work.example' }));
    expect(progress.getByRole('status')).toHaveTextContent('Reading the Outlook file');
    expect(progress.getByText(/This can take a long time for a big file\. You can close this/)).toBeInTheDocument();
    await user.click(progress.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    // Still running: the button waits, and another import is turned away.
    expect(section.getByRole('button', { name: 'Start import…' })).toBeDisabled();
    await user.click(await section.findByRole('button', { name: 'Import .ics…' }));
    expect(await screen.findByText('Another import is still running')).toBeInTheDocument();

    release();
    expect(await screen.findByText('Outlook import finished')).toBeInTheDocument();
    expect(
      screen.getByText('4 messages, 1 contact, 1 calendar event, 2 tasks and 1 note imported. 1 item left out.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(section.getByRole('button', { name: 'Start import…' })).toBeEnabled();

    await user.click(screen.getByRole('button', { name: 'Details' }));
    expect(within(screen.getByRole('dialog', { name: 'Outlook import finished' })).getByRole('button', { name: 'Open folder' })).toBeInTheDocument();
  });

  it('reports a failure after the dialog was closed as a toast', async () => {
    const { user, api, section } = await openSection();
    const release = api.holdFileTasks();
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    await user.click(
      within(await screen.findByRole('dialog', { name: /Importing from Outlook/ })).getByRole('button', { name: 'Close' }),
    );
    // The account goes away while the import runs.
    await api.removeAccount('work');
    release();
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Outlook import failed');
    expect(alert).toHaveTextContent('That account no longer exists.');
  });

  it('says so plainly while working offline', async () => {
    const { user, section } = await openSection({ settings: { ...MOCK_SETTINGS, workOffline: true } });
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    const alert = within(within(await screen.findByRole('dialog', { name: /Importing from Outlook/ })).getByRole('alert'));
    expect(alert.getByText("You're working offline")).toBeInTheDocument();
  });

  it('closes the dialog with Esc while it runs', async () => {
    const { user, api, section } = await openSection();
    const release = api.holdFileTasks();
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    await screen.findByRole('dialog', { name: /Importing from Outlook/ });
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    release();
    await waitFor(() => {
      expect(screen.getByText('Outlook import finished')).toBeInTheDocument();
    });
  });
});
