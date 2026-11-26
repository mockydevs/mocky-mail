import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { vi } from 'vitest';

import { MOCK_NOW, MOCK_SETTINGS } from '../api/mock/mockData';
import { createMockMailApi } from '../api/mock/mockMailApi';
import { messageWindowId } from '../features/reader/MessageWindow';
import { reorder } from '../features/mailbox/Sidebar';
import { accountFolders, reader, renderApp, rows } from '../test/renderApp';
import { App } from './App';

const panes = () => document.querySelector('main');
const settingsScreen = async (user: ReturnType<typeof renderApp>['user']) => {
  await rows();
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  return within(screen.getByRole('region', { name: 'Settings' }));
};

describe('reading pane position', () => {
  it('moves the reader below the list from the list header, and remembers it in Settings', async () => {
    const { user, api } = renderApp();
    await rows();
    expect(panes()).toHaveClass('panes');
    expect(panes()).not.toHaveClass('panes--bottom');
    await user.click(screen.getByRole('button', { name: 'Reading pane: Right' }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Bottom' }));
    expect(panes()).toHaveClass('panes--bottom');
    expect(reader()).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await api.getSettings()).readingPane).toBe('bottom');
    });
  });

  it('with the pane off, shows the list, then the message full width with Back', async () => {
    const { user } = renderApp({ settings: { ...MOCK_SETTINGS, readingPane: 'off' } });
    const list = await rows();
    expect(screen.queryByRole('article', { name: 'Reading pane' })).not.toBeInTheDocument();
    await user.click(list.getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    expect(screen.queryByRole('list', { name: 'Inbox messages' })).not.toBeInTheDocument();
    await user.click(within(reader()).getByRole('button', { name: 'Back to list' }));
    expect(await rows()).toBeTruthy();
    expect(screen.queryByRole('article', { name: 'Reading pane' })).not.toBeInTheDocument();
    // Esc goes back too.
    await user.click((await rows()).getByRole('button', { name: /Grace Njeri/ }));
    await within(reader()).findByRole('heading', { name: 'Photos from the weekend' });
    await user.keyboard('{Escape}');
    expect(await rows()).toBeTruthy();
  });

  it('is chosen in Settings › Appearance', async () => {
    const { user, api } = renderApp();
    const settings = await settingsScreen(user);
    await user.click(settings.getByRole('radio', { name: 'Off: open messages over the list' }));
    await vi.waitFor(async () => {
      expect((await api.getSettings()).readingPane).toBe('off');
    });
  });
});

describe('message in its own window', () => {
  it('reads the id from the address', () => {
    expect(messageWindowId('?message=m1')).toBe('m1');
    expect(messageWindowId('?message=a%2Fb')).toBe('a/b');
    expect(messageWindowId('')).toBeNull();
  });

  it('shows just the message, and replies in the same window', async () => {
    const api = createMockMailApi();
    const user = userEvent.setup();
    render(<App api={api} now={() => MOCK_NOW} messageId="m3" />);
    const pane = within(await screen.findByRole('article', { name: 'Reading pane' }));
    expect(await pane.findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    expect(screen.queryByRole('navigation', { name: 'Mailboxes' })).not.toBeInTheDocument();
    expect(screen.queryByRole('list', { name: /messages$/ })).not.toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'Reply' }));
    const compose = within(await screen.findByRole('form'));
    expect(compose.getByLabelText('Subject')).toHaveValue('Re: Lunch on Friday?');
    await user.click(compose.getByRole('button', { name: 'Close' }));
    await screen.findByRole('article', { name: 'Reading pane' });
  });

  it('flags and deletes from the window', async () => {
    const api = createMockMailApi();
    const user = userEvent.setup();
    render(<App api={api} now={() => MOCK_NOW} messageId="m2" />);
    const pane = within(await screen.findByRole('article', { name: 'Reading pane' }));
    await pane.findByRole('heading', { name: 'Nightly build passed on main' });
    await user.click(pane.getByRole('button', { name: 'Flag' }));
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).flagged).toBe(true);
    });
    await user.click(pane.getByRole('button', { name: 'Delete' }));
    expect(await screen.findByText('Moved to Trash')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close window' })).toBeInTheDocument();
    expect((await api.getMessage('m2', false)).folderId).toBe('work:trash');
  });

  it('has no such option on a phone', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    try {
      const { user } = renderApp({ phone: true });
      await user.click((await rows()).getByRole('button', { name: /Build server/ }));
      await user.click(await within(reader()).findByRole('button', { name: 'More actions' }));
      expect(within(reader()).queryByRole('button', { name: 'Open in new window' })).not.toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('favourites', () => {
  it('adds a folder from its context menu and lists it at the top of the sidebar', async () => {
    const { user, api } = renderApp();
    await rows();
    const clients = await accountFolders('Work').findByRole('button', { name: 'Clients' });
    fireEvent.contextMenu(clients, { clientX: 40, clientY: 200 });
    const menu = within(screen.getByRole('menu', { name: 'Clients folder' }));
    await user.click(menu.getByRole('menuitem', { name: 'Add to Favourites' }));
    const favourites = within(await screen.findByRole('list', { name: 'Favourites' }));
    await user.click(favourites.getByRole('button', { name: /^Clients \(Work\)/ }));
    expect(await rows('Clients')).toBeTruthy();
    expect((await api.listFolders('work')).find((f) => f.id === 'work:Clients')?.favorite).toBe(true);

    fireEvent.contextMenu(favourites.getByRole('button', { name: /^Clients/ }));
    await user.click(screen.getByRole('menuitem', { name: 'Remove from Favourites' }));
    await vi.waitFor(() => {
      expect(screen.queryByRole('list', { name: 'Favourites' })).not.toBeInTheDocument();
    });
  });

  it('closes the folder menu with Esc', async () => {
    const { user } = renderApp();
    await rows();
    fireEvent.contextMenu(await accountFolders('Work').findByRole('button', { name: 'Clients' }));
    expect(screen.getByRole('menuitem', { name: 'Add to Favourites' })).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('stars a folder in Manage folders', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(await accountFolders('Work').findByRole('button', { name: 'Manage folders' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Manage folders' }));
    await user.click(dialog.getByRole('button', { name: 'Add Clients / Acme to Favourites' }));
    expect(await dialog.findByRole('button', { name: 'Remove Clients / Acme from Favourites' })).toHaveAttribute('aria-pressed', 'true');
    expect((await api.listFolders('work')).find((f) => f.id === 'work:Clients/Acme')?.favorite).toBe(true);
  });
});

describe('account order', () => {
  const sidebarAccounts = () =>
    [...document.querySelectorAll('.account-toggle .nav-group__name')].map((el) => el.textContent);

  it('moves an account down in Settings › Accounts, and the sidebar follows', async () => {
    const { user, api } = renderApp();
    const settings = await settingsScreen(user);
    expect(sidebarAccounts()).toEqual(['Work', 'Personal']);
    expect(settings.getByRole('button', { name: 'Move you@work.example up' })).toBeDisabled();
    await user.click(settings.getByRole('button', { name: 'Move you@work.example down' }));
    await vi.waitFor(() => {
      expect(sidebarAccounts()).toEqual(['Personal', 'Work']);
    });
    expect((await api.listAccounts()).map((a) => a.id)).toEqual(['personal', 'work']);
  });

  it('drags an account to a new place in the sidebar', async () => {
    renderApp();
    await rows();
    const work = screen.getByRole('button', { name: /^Work \(/ });
    const personal = screen.getByRole('button', { name: /^Personal \(/ });
    const transfer = { setData: vi.fn(), effectAllowed: 'all', dropEffect: 'move' };
    fireEvent.dragStart(personal, { dataTransfer: transfer });
    fireEvent.dragOver(work, { dataTransfer: transfer });
    fireEvent.drop(work, { dataTransfer: transfer });
    await vi.waitFor(() => {
      expect(sidebarAccounts()).toEqual(['Personal', 'Work']);
    });
  });

  it('reorders ids', () => {
    expect(reorder(['a', 'b', 'c'], 'c', 'a')).toEqual(['c', 'a', 'b']);
    expect(reorder(['a', 'b', 'c'], 'a', 'c')).toEqual(['b', 'c', 'a']);
    expect(reorder(['a', 'b'], 'x', 'a')).toEqual(['a', 'b']);
  });
});

describe('keeping mail on this device', () => {
  it('sets how many months each account keeps', async () => {
    const { user, api } = renderApp();
    const settings = await settingsScreen(user);
    const work = within(settings.getByText('you@work.example').closest('li') as HTMLElement);
    const keep = work.getByRole('combobox', { name: 'Keep mail on this device' });
    expect(keep).toHaveValue('all');
    await user.selectOptions(keep, '6 months');
    expect(await work.findByText('Mail from the last 6 months is kept on this device.')).toBeInTheDocument();
    expect((await api.listAccounts()).find((a) => a.id === 'work')?.syncMonths).toBe(6);
  });
});

describe('work offline', () => {
  it('is switched on from the status bar: says so and turns the sync button off', async () => {
    const { user, api } = renderApp();
    await rows();
    const toggle = screen.getByRole('button', { name: 'Work offline' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('contentinfo')).toHaveTextContent('Working offline');
    const sync = screen.getByRole('button', { name: 'Check for new mail' });
    expect(sync).toBeDisabled();
    expect(sync).toHaveAccessibleDescription('Working offline. Switch off Work Offline to check for mail.');
    await vi.waitFor(async () => {
      expect((await api.getSettings()).workOffline).toBe(true);
    });
    await user.click(toggle);
    expect(screen.getByRole('button', { name: 'Check for new mail' })).toBeEnabled();
  });

  it('is in Settings too, for phones', async () => {
    const { user, api } = renderApp();
    const settings = await settingsScreen(user);
    await user.click(settings.getByRole('checkbox', { name: /Work offline/ }));
    expect(settings.getByText('Working offline. Messages you send wait in the Outbox.')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await api.getSettings()).workOffline).toBe(true);
    });
  });

  it('explains a refusal from the server as working offline', async () => {
    const { user, api } = renderApp({ settings: { ...MOCK_SETTINGS, workOffline: true } });
    await rows();
    api.failNext('moveMessages', { kind: 'connect', message: 'Working offline.' });
    await user.click(await within(reader()).findByRole('button', { name: 'Archive' }));
    expect(await screen.findByText("You're working offline")).toBeInTheDocument();
  });
});
