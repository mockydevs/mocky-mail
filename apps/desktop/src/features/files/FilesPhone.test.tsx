import { act, configure, fireEvent, screen, within } from '@testing-library/react';
import type * as TauriCore from '@tauri-apps/api/core';
import { afterEach, beforeEach, vi } from 'vitest';

import type { OpenedFile } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

// Other work on this machine can slow the app down; give each step time.
configure({ asyncUtilTimeout: 4000 });

/** The Android Back button, as the app sees it through Tauri. */
const back = vi.hoisted(() => ({ press: null as (() => void) | null }));
vi.mock('@tauri-apps/api/core', async (original) => ({
  ...(await original<typeof TauriCore>()),
  isTauri: () => true,
}));
vi.mock('@tauri-apps/api/app', () => ({
  onBackButtonPress: (handler: () => void) => {
    back.press = handler;
    return Promise.resolve({
      unregister: () => {
        if (back.press === handler) back.press = null;
        return Promise.resolve();
      },
    });
  },
}));

const EML: OpenedFile = { kind: 'eml', path: '/storage/emulated/0/Download/drawings.eml', name: 'drawings.eml' };
const SUBJECT = 'Signed drawings for the Kilimani house';

beforeEach(() => {
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: query.startsWith('(max-width'),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
  back.press = null;
});

async function pressBack() {
  await vi.waitFor(() => {
    expect(back.press).not.toBeNull();
  });
  act(() => {
    back.press?.();
  });
}

async function openDrawer(user: ReturnType<typeof renderApp>['user']) {
  await user.click(screen.getByRole('button', { name: 'Folders and accounts' }));
  return within(screen.getByRole('navigation', { name: 'Mailboxes' }));
}

describe('import and export on a phone', () => {
  it('imports messages from Settings and opens the folder', async () => {
    const { user } = renderApp({ phone: true });
    await rows();
    await user.click((await openDrawer(user)).getByRole('button', { name: 'Settings' }));
    const section = within(screen.getByRole('region', { name: 'Import and export' }));
    await user.click(section.getByRole('button', { name: 'Import messages…' }));
    const picker = within(screen.getByRole('dialog', { name: 'Import messages into…' }));
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Work: Clients' }));
    const summary = within(await screen.findByRole('dialog', { name: 'Messages imported' }));
    await user.click(summary.getByRole('button', { name: 'Open folder' }));
    expect(await (await rows('Clients')).findByRole('button', { name: /Invoice INV-0098/ })).toBeInTheDocument();
  });

  it('imports into a folder from a long press in the folder menu, closing the menu drawer', async () => {
    const { user } = renderApp({ phone: true });
    await rows();
    const drawer = await openDrawer(user);
    fireEvent.contextMenu(drawer.getByRole('button', { name: 'Clients' }));
    await user.click(screen.getByRole('menuitem', { name: 'Import messages into this folder…' }));
    expect(screen.queryByRole('button', { name: 'Close menu' })).not.toBeInTheDocument();
    expect(await screen.findByRole('dialog', { name: 'Messages imported' })).toBeInTheDocument();
  });

  it('saves long-pressed messages as .eml files', async () => {
    const { user, api } = renderApp({ phone: true });
    const spy = vi.spyOn(api, 'exportMail');
    fireEvent.contextMenu((await rows()).getByRole('button', { name: /Brian Otieno/ }));
    const bar = within(screen.getByRole('toolbar', { name: 'Selected messages' }));
    await user.click(bar.getByRole('button', { name: 'Save as .eml (zip)…' }));
    expect(spy).toHaveBeenCalledWith({ ids: ['m3'] }, 'eml');
    expect(await screen.findByText('/home/you/Documents/Messages (1).zip')).toBeInTheDocument();
  });

  it('runs an Outlook import that keeps going after Back closes its dialog', async () => {
    const { user, api } = renderApp({ phone: true });
    await rows();
    await user.click((await openDrawer(user)).getByRole('button', { name: 'Settings' }));
    const section = within(screen.getByRole('region', { name: 'Import and export' }));
    const release = api.holdFileTasks();
    await user.click(section.getByRole('button', { name: 'Start import…' }));
    await screen.findByRole('dialog', { name: /Importing from Outlook/ });
    await pressBack();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    // Back closed only the dialog, not Settings.
    expect(screen.getByRole('region', { name: 'Import and export' })).toBeInTheDocument();
    release();
    expect(await screen.findByText('Outlook import finished')).toBeInTheDocument();
  });

  it('imports contacts from the contacts menu', async () => {
    const { user, api } = renderApp({ phone: true, contacts: [] });
    await rows();
    await user.click((await openDrawer(user)).getByRole('button', { name: 'Contacts' }));
    const spy = vi.spyOn(api, 'importContactsFile');
    await user.click(screen.getByRole('button', { name: 'More contact options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Import contacts (.vcf)…' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Import contacts (.vcf)' })).getByRole('button', { name: 'Choose file…' }));
    expect(spy).toHaveBeenCalledWith('work');
    expect(await screen.findByRole('dialog', { name: 'Contacts imported' })).toBeInTheDocument();
  });
});

describe('opened files on a phone', () => {
  it('shows a shared .eml file and replies to it', async () => {
    const { user } = renderApp({ phone: true, openedFiles: [EML] });
    const message = within(await screen.findByRole('dialog', { name: SUBJECT }));
    await user.click(message.getByRole('button', { name: 'Reply' }));
    expect(await screen.findByRole('form', { name: 'Reply' })).toBeInTheDocument();
  });

  it('closes the viewer with Back, then the folder picker first when it is open', async () => {
    const { user } = renderApp({ phone: true, openedFiles: [EML] });
    const message = within(await screen.findByRole('dialog', { name: SUBJECT }));
    await user.click(message.getByRole('button', { name: 'Add to folder…' }));
    expect(screen.getByRole('dialog', { name: 'Add the email to…' })).toBeInTheDocument();
    await pressBack();
    expect(screen.queryByRole('dialog', { name: 'Add the email to…' })).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: SUBJECT })).toBeInTheDocument();
    await pressBack();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(await rows()).toBeTruthy();
  });

  it('adds a shared contacts file', async () => {
    const { user, api } = renderApp({
      phone: true,
      openedFiles: [{ kind: 'vcf', path: 'content://downloads/team.vcf', name: 'team.vcf' }],
    });
    const spy = vi.spyOn(api, 'importOpenedFile');
    const dialog = within(await screen.findByRole('dialog', { name: 'Add contacts from team.vcf' }));
    await user.click(dialog.getByRole('button', { name: 'Add contacts' }));
    expect(spy).toHaveBeenCalledWith('content://downloads/team.vcf', { accountId: 'work' });
    expect(await screen.findByRole('dialog', { name: 'Contacts added' })).toBeInTheDocument();
  });
});
