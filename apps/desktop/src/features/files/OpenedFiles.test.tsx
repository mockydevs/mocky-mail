import { act, configure, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import type { OpenedFile } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

// Other work on this machine can slow the app down; give each step time.
configure({ asyncUtilTimeout: 4000 });

const EML: OpenedFile = { kind: 'eml', path: '/home/you/Downloads/drawings.eml', name: 'drawings.eml' };
const ICS: OpenedFile = { kind: 'ics', path: '/home/you/Downloads/site-meeting.ics', name: 'site-meeting.ics' };
const VCF: OpenedFile = { kind: 'vcf', path: '/home/you/Downloads/team.vcf', name: 'team.vcf' };
const SUBJECT = 'Signed drawings for the Kilimani house';

const viewer = async () => within(await screen.findByRole('dialog', { name: SUBJECT }));

describe('an opened .eml file', () => {
  it('shows the message the app was started with, and replies to it', async () => {
    const { user } = renderApp({ openedFiles: [EML] });
    const message = await viewer();
    expect(message.getByText(/From the file drawings\.eml/)).toBeInTheDocument();
    expect(message.getByText('Amina Hassan')).toBeInTheDocument();
    expect(message.getByText(/The client approved the kitchen layout/)).toBeInTheDocument();
    expect(message.getByText('Drawings-signed.pdf')).toBeInTheDocument();

    await user.click(message.getByRole('button', { name: 'Reply' }));
    expect(screen.queryByRole('dialog', { name: SUBJECT })).not.toBeInTheDocument();
    const compose = within(await screen.findByRole('form', { name: 'Reply' }));
    expect(compose.getByRole('textbox', { name: 'Subject' })).toHaveValue(`Re: ${SUBJECT}`);
    expect(within(compose.getByRole('list', { name: 'To recipients' })).getByText(/Amina Hassan/)).toBeInTheDocument();
  });

  it('forwards it with its attachment', async () => {
    const { user } = renderApp({ openedFiles: [EML] });
    await user.click((await viewer()).getByRole('button', { name: 'Forward' }));
    const compose = within(await screen.findByRole('form', { name: 'Forward' }));
    expect(compose.getByRole('textbox', { name: 'Subject' })).toHaveValue(`Fwd: ${SUBJECT}`);
    expect(await compose.findByText('Drawings-signed.pdf')).toBeInTheDocument();
  });

  it('adds it to a folder and opens that folder from the toast', async () => {
    const { user, api } = renderApp({ openedFiles: [EML] });
    const spy = vi.spyOn(api, 'importMessageFile');
    await user.click((await viewer()).getByRole('button', { name: 'Add to folder…' }));
    const picker = within(screen.getByRole('dialog', { name: 'Add the email to…' }));
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Work: Clients' }));
    expect(spy).toHaveBeenCalledWith(EML.path, 'work:Clients');
    expect(await screen.findByText('Added to Clients')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open folder' }));
    expect(await (await rows('Clients')).findByRole('button', { name: new RegExp(SUBJECT) })).toBeInTheDocument();
  });

  it('says plainly when adding it fails', async () => {
    const { user, api } = renderApp({ openedFiles: [EML] });
    api.failNext('importMessageFile', { kind: 'connect', message: 'Connection timed out.' });
    const message = await viewer();
    await user.click(message.getByRole('button', { name: 'Add to folder…' }));
    await user.click(
      within(screen.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Work: Inbox' }),
    );
    const alert = within(await message.findByRole('alert'));
    expect(alert.getByText("Couldn't reach the mail server")).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: SUBJECT })).toBeInTheDocument();
  });

  it('saves an attachment, or hides saving when the backend only saves stored mail', async () => {
    const { user, api } = renderApp({ openedFiles: [EML] });
    const message = await viewer();
    await user.click(message.getByRole('button', { name: 'Save Drawings-signed.pdf' }));
    expect(await message.findByText('Saved: /home/you/Downloads/Drawings-signed.pdf')).toBeInTheDocument();

    api.failNext('saveAttachment', { kind: 'not_found', message: 'That message is no longer available.' });
    await user.click(message.getByRole('button', { name: 'Save Drawings-signed.pdf' }));
    expect(await message.findByText(/add the email to a folder first/)).toBeInTheDocument();
    expect(message.queryByRole('button', { name: 'Save Drawings-signed.pdf' })).not.toBeInTheDocument();
  });

  it('opens one sent while the app runs, after the one before it is closed', async () => {
    const { user, api } = renderApp();
    await rows();
    act(() => {
      api.emit('mail://open-file', { file: EML });
      api.emit('mail://open-file', { file: VCF });
    });
    const message = await viewer();
    await user.click(message.getByRole('button', { name: 'Close' }));
    expect(await screen.findByRole('dialog', { name: 'Add contacts from team.vcf' })).toBeInTheDocument();
  });

  it('explains a file that cannot be read', async () => {
    const { user } = renderApp({ openedFiles: [{ kind: 'eml', path: '/tmp/broken.eml', name: 'broken.eml' }] });
    const dialog = within(await screen.findByRole('dialog', { name: 'broken.eml' }));
    const alert = within(await dialog.findByRole('alert'));
    expect(alert.getByText("This email couldn't be opened")).toBeInTheDocument();
    expect(alert.getByText(/broken\.eml could not be read/)).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('an opened .ics or .vcf file', () => {
  it('adds a calendar file to the chosen calendar', async () => {
    const { user, api } = renderApp({ openedFiles: [ICS] });
    const spy = vi.spyOn(api, 'importOpenedFile');
    const dialog = within(await screen.findByRole('dialog', { name: 'Add site-meeting.ics to calendar' }));
    await user.selectOptions(await dialog.findByRole('combobox', { name: 'Calendar' }), 'Family (you@personal.example)');
    await user.click(dialog.getByRole('button', { name: 'Add to calendar' }));
    expect(spy).toHaveBeenCalledWith(ICS.path, { calendarId: 'personal:personal' });
    const summary = within(await screen.findByRole('dialog', { name: 'Events added' }));
    expect(summary.getByRole('status')).toHaveTextContent('2 events imported.');
    await user.click(summary.getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('adds a contacts file to this device or an account', async () => {
    const { user, api } = renderApp({ openedFiles: [VCF] });
    const spy = vi.spyOn(api, 'importOpenedFile');
    const dialog = within(await screen.findByRole('dialog', { name: 'Add contacts from team.vcf' }));
    await user.selectOptions(dialog.getByRole('combobox', { name: 'Add them to' }), 'This device only');
    await user.click(dialog.getByRole('button', { name: 'Add contacts' }));
    expect(spy).toHaveBeenCalledWith(VCF.path, { accountId: null });
    const summary = within(await screen.findByRole('dialog', { name: 'Contacts added' }));
    expect(summary.getByRole('status')).toHaveTextContent('2 contacts imported.');
    expect((await api.listContacts('Faith')).map((c) => c.accountId)).toEqual([null]);
  });

  it('shows why a contacts file could not be added', async () => {
    const { user, api } = renderApp({ openedFiles: [VCF] });
    api.failNext('importOpenedFile', { kind: 'invalid', message: 'team.vcf has no contacts in it.' });
    const dialog = within(await screen.findByRole('dialog', { name: 'Add contacts from team.vcf' }));
    await user.click(dialog.getByRole('button', { name: 'Add contacts' }));
    const alert = within(within(await screen.findByRole('dialog', { name: /Adding contacts to/ })).getByRole('alert'));
    expect(alert.getByText("This file couldn't be used")).toBeInTheDocument();
    expect(alert.getByText('team.vcf has no contacts in it.')).toBeInTheDocument();
  });

  it('can be cancelled without adding anything', async () => {
    const { user, api } = renderApp({ openedFiles: [ICS] });
    const spy = vi.spyOn(api, 'importOpenedFile');
    const dialog = within(await screen.findByRole('dialog', { name: 'Add site-meeting.ics to calendar' }));
    await user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
  });
});
