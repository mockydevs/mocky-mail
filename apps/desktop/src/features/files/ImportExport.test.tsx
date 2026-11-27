import { act, configure, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

import { MOCK_SETTINGS } from '../../api/mock/mockData';
import { accountFolders, accountToggle, renderApp, rows } from '../../test/renderApp';

// Other work on this machine can slow the app down; give each step time.
configure({ asyncUtilTimeout: 4000 });

const dialog = (name: string | RegExp) => within(screen.getByRole('dialog', { name }));
const findDialog = async (name: string | RegExp) => within(await screen.findByRole('dialog', { name }));

async function folderMenu(account: string, folder: string) {
  const button = await accountFolders(account).findByRole('button', { name: folder });
  fireEvent.contextMenu(button, { clientX: 40, clientY: 200 });
  return within(screen.getByRole('menu', { name: `${folder} folder` }));
}

describe('importing and exporting mail from the folder menu', () => {
  it('imports into a folder with a progress dialog, then a summary that opens the folder', async () => {
    const { user, api } = renderApp();
    await rows();
    const release = api.holdFileTasks();
    const spy = vi.spyOn(api, 'importMailFiles');
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Import messages into this folder…' }));
    expect(spy).toHaveBeenCalledWith('work:Clients');

    // First step: nothing counted yet, so the bar just moves.
    const progress = await findDialog('Importing messages into Clients');
    expect(progress.getByRole('status')).toHaveTextContent('Reading files');
    expect(progress.getByRole('progressbar')).not.toHaveAttribute('value');

    // Counted steps fill the bar.
    act(() => {
      api.emit('mail://import-progress', { phase: 'Uploading messages', done: 2, total: 3 });
    });
    expect(progress.getByRole('status')).toHaveTextContent('Uploading messages · 2 of 3');
    expect(progress.getByRole('progressbar')).toHaveAttribute('value', '2');
    expect(progress.getByRole('progressbar')).toHaveAttribute('max', '3');
    release();

    const summary = await findDialog('Messages imported');
    expect(summary.getByRole('status')).toHaveTextContent('3 messages imported.');
    expect(summary.getByText('1 item left out.')).toBeInTheDocument();
    const why = summary.getByText('Why one item was left out');
    expect(why.closest('details')).not.toHaveAttribute('open');
    await user.click(why);
    expect(why.closest('details')).toHaveAttribute('open');
    expect(summary.getByText(/newsletter\.eml was left out/)).toBeVisible();

    await user.click(summary.getByRole('button', { name: 'Open folder' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(await (await rows('Clients')).findByRole('button', { name: /Site visit notes, Karen/ })).toBeInTheDocument();
  });

  it('exports a folder as .mbox and says where it went', async () => {
    const { user, api } = renderApp();
    await rows();
    const spy = vi.spyOn(api, 'exportMail');
    await user.click(accountToggle('Personal'));
    await user.click((await folderMenu('Personal', 'Travel')).getByRole('menuitem', { name: 'Export folder as .mbox…' }));
    expect(spy).toHaveBeenCalledWith({ folderId: 'personal:Travel' }, 'mbox');
    expect(await screen.findByText('/home/you/Documents/Travel.mbox')).toBeInTheDocument();
    expect(screen.getByText('Saved')).toBeInTheDocument();
  });

  it('does nothing when the file picker is cancelled', async () => {
    const { user, api } = renderApp();
    await rows();
    api.cancelNextPick();
    const spy = vi.spyOn(api, 'importMailFiles');
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Import messages into this folder…' }));
    await waitFor(() => {
      expect(spy).toHaveBeenCalled();
    });
    await act(async () => {
      await spy.mock.results[0]?.value;
    });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByText('Saved')).not.toBeInTheDocument();
  });

  it('saves ticked messages as .eml files in a zip from the selection bar', async () => {
    const { user, api } = renderApp();
    await rows();
    const spy = vi.spyOn(api, 'exportMail');
    await user.click(screen.getByRole('checkbox', { name: /Select Wanjiru Kamau/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    const bar = within(screen.getByRole('toolbar', { name: 'Selected messages' }));
    await user.click(bar.getByRole('button', { name: 'Save as .eml (zip)…' }));
    const [target, format] = spy.mock.calls[0] ?? [];
    expect(format).toBe('eml');
    const ids = target && 'ids' in target ? target.ids : [];
    expect(ids.includes('m1') && ids.includes('m2')).toBe(true);
    expect(await screen.findByText(/\/home\/you\/Documents\/Messages \(\d+\)\.zip/)).toBeInTheDocument();
  });
});

describe('calendar import and export', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-11-13T09:00:00Z'));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  async function openCalendar() {
    const ctx = renderApp();
    await rows();
    await ctx.user.click(screen.getByRole('button', { name: 'Calendar' }));
    const region = within(await screen.findByRole('region', { name: 'November 2026' }));
    await region.findAllByRole('button', { name: /Team stand-up/ });
    return { ...ctx, region };
  }

  it('imports an .ics file into the chosen calendar from the calendar menu', async () => {
    const { user, api, region } = await openCalendar();
    const spy = vi.spyOn(api, 'importCalendarFile');
    await user.click(region.getByRole('button', { name: 'Import and export calendars' }));
    const menu = within(screen.getByRole('menu', { name: 'Import and export calendars' }));
    const family = within(menu.getByRole('group', { name: 'Family (you@personal.example)' }));
    await user.click(family.getByRole('menuitem', { name: 'Import .ics…' }));
    expect(spy).toHaveBeenCalledWith('personal:personal');

    const summary = await findDialog('Events imported');
    expect(summary.getByRole('status')).toHaveTextContent('2 events imported.');
    expect(summary.queryByRole('button', { name: 'Open folder' })).not.toBeInTheDocument();
    await user.click(summary.getByRole('button', { name: 'Done' }));
    expect(await region.findAllByRole('button', { name: /Board meeting/ })).not.toHaveLength(0);
  });

  it('exports a calendar as .ics', async () => {
    const { user, api, region } = await openCalendar();
    const spy = vi.spyOn(api, 'exportCalendar');
    await user.click(region.getByRole('button', { name: 'Import and export calendars' }));
    const work = within(screen.getByRole('group', { name: 'Personal Calendar (you@work.example)' }));
    await user.click(work.getByRole('menuitem', { name: 'Export calendar…' }));
    expect(spy).toHaveBeenCalledWith('work:personal');
    expect(await screen.findByText('/home/you/Documents/Personal Calendar.ics')).toBeInTheDocument();
  });
});

describe('contacts import and export', () => {
  async function openContacts() {
    const ctx = renderApp({ contacts: [] });
    await rows();
    await ctx.user.click(screen.getByRole('button', { name: 'Contacts' }));
    const region = within(screen.getByRole('region', { name: 'Contacts' }));
    return { ...ctx, region };
  }

  it('imports a .vcf file to this device from the contacts menu', async () => {
    const { user, api, region } = await openContacts();
    const spy = vi.spyOn(api, 'importContactsFile');
    await user.click(region.getByRole('button', { name: 'More contact options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Import contacts (.vcf)…' }));
    const ask = dialog('Import contacts (.vcf)');
    await user.selectOptions(ask.getByRole('combobox', { name: 'Add them to' }), 'This device only');
    await user.click(ask.getByRole('button', { name: 'Choose file…' }));
    expect(spy).toHaveBeenCalledWith(null);

    const summary = await findDialog('Contacts imported');
    expect(summary.getByRole('status')).toHaveTextContent('2 contacts imported.');
    await user.click(summary.getByRole('button', { name: 'Done' }));
    const list = within(await region.findByRole('list', { name: 'Contacts' }));
    expect(await list.findByRole('button', { name: /Grace Otieno/ })).toBeInTheDocument();
  });

  it('exports every contact, or one address book', async () => {
    const { user, api, region } = await openContacts();
    const spy = vi.spyOn(api, 'exportContacts');
    await user.click(region.getByRole('button', { name: 'More contact options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Export contacts…' }));
    const ask = dialog('Export contacts');
    expect(ask.getByRole('combobox', { name: 'Contacts from' })).toHaveDisplayValue('All contacts');
    await user.click(ask.getByRole('button', { name: 'Save…' }));
    expect(spy).toHaveBeenLastCalledWith('*');
    expect(await screen.findByText('/home/you/Documents/All contacts.vcf')).toBeInTheDocument();

    await user.click(region.getByRole('button', { name: 'More contact options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Export contacts…' }));
    const again = dialog('Export contacts');
    await user.selectOptions(again.getByRole('combobox', { name: 'Contacts from' }), 'Work (you@work.example)');
    await user.click(again.getByRole('button', { name: 'Save…' }));
    expect(spy).toHaveBeenLastCalledWith('work');
    expect(await screen.findByText('/home/you/Documents/Work.vcf')).toBeInTheDocument();
  });

  it('cancels without asking the backend', async () => {
    const { user, api, region } = await openContacts();
    const spy = vi.spyOn(api, 'importContactsFile');
    await user.click(region.getByRole('button', { name: 'More contact options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Import contacts (.vcf)…' }));
    await user.click(dialog('Import contacts (.vcf)').getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
  });
});

describe('Settings › Import and export', () => {
  async function openSection(options: Parameters<typeof renderApp>[0] = {}) {
    const ctx = renderApp(options);
    await rows();
    await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
    const section = within(screen.getByRole('region', { name: 'Import and export' }));
    return { ...ctx, section };
  }

  it('imports messages into a folder picked from every account', async () => {
    const { user, api, section } = await openSection();
    const spy = vi.spyOn(api, 'importMailFiles');
    await user.click(section.getByRole('button', { name: 'Import messages…' }));
    const picker = dialog('Import messages into…');
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Personal: Travel' }));
    expect(spy).toHaveBeenCalledWith('personal:Travel');
    const summary = await findDialog('Messages imported');
    expect(summary.getByRole('status')).toHaveTextContent('3 messages imported.');
    await user.click(summary.getByRole('button', { name: 'Open folder' }));
    expect(await (await rows('Travel')).findByRole('button', { name: /Holiday plans/ })).toBeInTheDocument();
  });

  it('exports a folder as .mbox', async () => {
    const { user, api, section } = await openSection();
    const spy = vi.spyOn(api, 'exportMail');
    await user.click(section.getByRole('button', { name: 'Export a folder as .mbox…' }));
    const picker = dialog('Export which folder?');
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Work: Inbox' }));
    expect(spy).toHaveBeenCalledWith({ folderId: 'work:inbox' }, 'mbox');
    expect(await screen.findByText('/home/you/Documents/Inbox.mbox')).toBeInTheDocument();
  });

  it('imports and exports the chosen calendar', async () => {
    const { user, api, section } = await openSection();
    const importing = vi.spyOn(api, 'importCalendarFile');
    const exporting = vi.spyOn(api, 'exportCalendar');
    await user.selectOptions(await section.findByRole('combobox', { name: 'Calendar' }), 'Family (you@personal.example)');
    await user.click(section.getByRole('button', { name: 'Import .ics…' }));
    expect(importing).toHaveBeenCalledWith('personal:personal');
    await user.click((await findDialog('Events imported')).getByRole('button', { name: 'Done' }));
    await user.click(section.getByRole('button', { name: 'Export calendar…' }));
    expect(exporting).toHaveBeenCalledWith('personal:personal');
    expect(await screen.findByText('/home/you/Documents/Family.ics')).toBeInTheDocument();
  });

  it('imports and exports contacts of the chosen address book, or all of them', async () => {
    const { user, api, section } = await openSection();
    const importing = vi.spyOn(api, 'importContactsFile');
    const exporting = vi.spyOn(api, 'exportContacts');
    const book = section.getByRole('combobox', { name: 'Address book' });
    expect(book).toHaveDisplayValue('Work (you@work.example)');
    await user.click(section.getByRole('button', { name: 'Import contacts (.vcf)…' }));
    expect(importing).toHaveBeenLastCalledWith('work');
    await user.click((await findDialog('Contacts imported')).getByRole('button', { name: 'Done' }));

    await user.selectOptions(book, 'This device only');
    await user.click(section.getByRole('button', { name: 'Export these contacts…' }));
    expect(exporting).toHaveBeenLastCalledWith(null);
    expect(await screen.findByText('/home/you/Documents/This device.vcf')).toBeInTheDocument();
    await user.click(section.getByRole('button', { name: 'Export all contacts…' }));
    expect(exporting).toHaveBeenLastCalledWith('*');
  });
});

describe('import errors in plain words', () => {
  it('says when there is no connection', async () => {
    const { user, api } = renderApp();
    await rows();
    api.failNext('importMailFiles', { kind: 'connect', message: 'Connection refused.' });
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Import messages into this folder…' }));
    const alert = within((await findDialog('Importing messages into Clients')).getByRole('alert'));
    expect(alert.getByText("Couldn't reach the mail server")).toBeInTheDocument();
    expect(alert.getByText(/Check that you're online, then try again\. Connection refused\./)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'OK' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('says so while working offline', async () => {
    const { user } = renderApp({ settings: { ...MOCK_SETTINGS, workOffline: true } });
    await rows();
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Import messages into this folder…' }));
    const alert = within((await findDialog('Importing messages into Clients')).getByRole('alert'));
    expect(alert.getByText("You're working offline")).toBeInTheDocument();
    expect(alert.getByText(/Turn off Work Offline/)).toBeInTheDocument();
  });

  it('says when the server refused', async () => {
    const { user, api } = renderApp();
    await rows();
    api.failNext('importMailFiles', { kind: 'auth', message: 'The server rejected the password.' });
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Import messages into this folder…' }));
    const alert = within((await findDialog('Importing messages into Clients')).getByRole('alert'));
    expect(alert.getByText('The mail server refused')).toBeInTheDocument();
    expect(alert.getByText(/Sign in again in Settings › Accounts/)).toBeInTheDocument();
  });

  it('says when a file could not be read', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const section = within(screen.getByRole('region', { name: 'Import and export' }));
    api.failNext('importCalendarFile', { kind: 'invalid', message: 'party.ics could not be read. It may be damaged.' });
    await user.click(await section.findByRole('button', { name: 'Import .ics…' }));
    const alert = within((await findDialog('Importing events into Personal Calendar')).getByRole('alert'));
    expect(alert.getByText("This file couldn't be used")).toBeInTheDocument();
    expect(alert.getByText('party.ics could not be read. It may be damaged.')).toBeInTheDocument();
  });

  it('says when saving on this device failed', async () => {
    const { user, api } = renderApp();
    await rows();
    api.failNext('exportMail', { kind: 'storage', message: 'Disk full.' });
    await user.click((await folderMenu('Work', 'Clients')).getByRole('menuitem', { name: 'Export folder as .mbox…' }));
    const alert = within((await findDialog('Saving Clients as .mbox')).getByRole('alert'));
    expect(alert.getByText("Couldn't save on this device")).toBeInTheDocument();
    expect(alert.getByText(/Disk full\./)).toBeInTheDocument();
  });
});
