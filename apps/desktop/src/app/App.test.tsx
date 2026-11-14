import { act, cleanup, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_ACCOUNTS, MOCK_NOW } from '../api/mock/mockData';
import * as chimeModule from '../features/notifications/chime';
import type { MockMessage } from '../api/mock/mockMailApi';
import { accountFolders, deferred, reader, renderApp, rows, unified, accountToggle } from '../test/renderApp';

describe('mailbox', () => {
  it('opens the unified inbox across accounts without marking anything read', async () => {
    renderApp();
    const list = await rows();
    expect(list.getAllByRole('button').map((b) => b.dataset['id'])).toEqual(['m1', 'm2', 'm3', 'm5', 'm6', 'm7']);
    expect(await within(reader()).findByRole('heading', { name: /Westlands office fit-out/ })).toBeInTheDocument();
    expect(await unified().findByRole('button', { name: 'Inbox 4 unread' })).toHaveAttribute('aria-current', 'page');
    expect(accountFolders('Work').getByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
    // Only one account is open at a time; the others show their inbox count on one row.
    expect(accountToggle('Work')).toHaveAttribute('aria-expanded', 'true');
    expect(accountToggle('Personal')).toHaveAttribute('aria-expanded', 'false');
    expect(accountToggle('Personal')).toHaveAccessibleName('Personal (you@personal.example), 1 unread');
    expect(screen.queryByRole('list', { name: 'Personal' })).not.toBeInTheDocument();
  });

  it('opens one account at a time and shows its inbox', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(accountToggle('Personal'));
    expect(accountToggle('Personal')).toHaveAttribute('aria-expanded', 'true');
    expect(accountToggle('Work')).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('list', { name: 'Work' })).not.toBeInTheDocument();
    expect(accountFolders('Personal').getByRole('button', { name: 'Inbox 1 unread' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // Clicking the open account folds it away; nothing else changes.
    await user.click(accountToggle('Personal'));
    expect(accountToggle('Personal')).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('list', { name: 'Personal' })).not.toBeInTheDocument();
    expect(accountToggle('Work')).toHaveAttribute('aria-expanded', 'false');
  });

  it('lists special folders first in each account', async () => {
    renderApp();
    await rows();
    await accountFolders('Work').findByRole('button', { name: 'Spam' });
    const names = accountFolders('Work')
      .getAllByRole('button')
      .filter((b) => !b.hasAttribute('aria-expanded'))
      .map((b) => b.textContent);
    expect(names.slice(0, 6)).toEqual(['Inbox 3 unread', 'Drafts', 'Sent', 'Archive', 'Spam', 'Trash']);
  });

  it('marks a message read only when the user opens it', async () => {
    const { user } = renderApp();
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
  });

  it('opens a single folder of one account', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Clients/ }));
    const list = await rows('Clients');
    expect(list.getAllByRole('button')).toHaveLength(1);
    expect(screen.getByText(/1 message, Work/)).toBeInTheDocument();
  });

  it('shows an empty state and no message selected for an empty folder', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await accountFolders('Work').findByRole('button', { name: 'Spam' }));
    expect(await screen.findByText('No spam')).toBeInTheDocument();
    expect(within(reader()).getByText('No message selected')).toBeInTheDocument();
  });

  it('filters unread and keeps the opened message visible after it is read', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Unread' }));
    expect((await rows()).getAllByRole('button')).toHaveLength(4);
    await user.click((await rows()).getByRole('button', { name: /Brian Otieno/ }));
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
    const list = await rows();
    expect(list.getAllByRole('button')).toHaveLength(4);
    expect(list.getByRole('button', { current: true })).toHaveTextContent('Brian Otieno');
  });

  it('debounces search so typing sends one query', async () => {
    const { user, api } = renderApp();
    await rows();
    const spy = vi.spyOn(api, 'listMessages');
    await user.type(screen.getByRole('searchbox', { name: 'Search mail' }), 'naivasha');
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(1);
    });
    const searches = spy.mock.calls.map(([q]) => q.search).filter((s) => s !== null);
    expect(searches).toEqual(['naivasha']);
  });

  const bulk = (count: number): MockMessage[] =>
    Array.from({ length: count }, (_, i) => ({
      id: `bulk${String(i)}`, accountId: 'work', folderId: 'work:inbox',
      from: { name: `Sender ${String(i)}`, address: `s${String(i)}@x.example` }, to: [],
      subject: `Message ${String(i)}`, preview: '', date: MOCK_NOW - i * 60, unread: false, flagged: false,
      answered: false, hasAttachments: false, cc: [], replyTo: [], html: null, text: 'x',
      blockedRemoteImages: 0, attachments: [], messageIdHeader: null, inReplyTo: null, references: [],
    }));

  it('pages through long folders with a Load more button', async () => {
    const { user } = renderApp({ messages: bulk(120) });
    expect((await rows()).getAllByRole('button')).toHaveLength(50);
    await user.click(screen.getByRole('button', { name: 'Load more (70 left)' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(100);
    });
    await user.click(screen.getByRole('button', { name: 'Load more (20 left)' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(120);
    });
    expect(screen.queryByRole('button', { name: /Load more/ })).not.toBeInTheDocument();
  });

  it('loads more as the list scrolls, observing the list container, not the window', async () => {
    const observers: { root: Element | Document | null | undefined; fire: () => void; live: boolean }[] = [];
    class FakeObserver {
      private record: (typeof observers)[number];
      constructor(cb: IntersectionObserverCallback, options?: IntersectionObserverInit) {
        this.record = {
          root: options?.root,
          live: true,
          fire: () => {
            cb([{ isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
          },
        };
        observers.push(this.record);
      }
      observe() {
        return undefined;
      }
      disconnect() {
        this.record.live = false;
      }
    }
    vi.stubGlobal('IntersectionObserver', FakeObserver);
    try {
      renderApp({ messages: bulk(120) });
      expect((await rows()).getAllByRole('button')).toHaveLength(50);
      const live = () => {
        const found = observers.filter((o) => o.live).at(-1);
        if (!found) throw new Error('no live observer');
        return found;
      };
      await vi.waitFor(() => {
        expect(live().root).toBe(screen.getByTestId('list-scroll'));
      });
      act(() => {
        live().fire();
      });
      await vi.waitFor(async () => {
        expect((await rows()).getAllByRole('button')).toHaveLength(100);
      });
      await vi.waitFor(() => {
        expect(live().root).toBe(screen.getByTestId('list-scroll'));
      });
      act(() => {
        live().fire();
      });
      await vi.waitFor(async () => {
        expect((await rows()).getAllByRole('button')).toHaveLength(120);
      });
      expect(observers.every((o) => o.root === screen.getByTestId('list-scroll'))).toBe(true);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('refetches when the backend says mail changed', async () => {
    const { api } = renderApp();
    await rows();
    await api.setFlags(['m2', 'm5'], { seen: true });
    act(() => {
      api.emit('mail://changed', { accountId: 'work', folderIds: ['work:inbox'] });
    });
    expect(await unified().findByRole('button', { name: 'Inbox 2 unread' })).toBeInTheDocument();
  });

  it('rolls back an optimistic flag when the backend refuses', async () => {
    const { user, api } = renderApp();
    await rows();
    const pending = deferred();
    vi.spyOn(api, 'setFlags').mockImplementationOnce(() => pending.promise);
    const flag = await within(reader()).findByRole('button', { name: 'Flag' });
    await user.click(flag);
    expect(within(reader()).getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'true');
    await act(async () => {
      pending.reject({ kind: 'connect', message: 'The server did not answer.' });
      await Promise.resolve();
    });
    expect(within(reader()).getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'false');
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Could not flag: Could not reach the mail server');
    expect(alert).toHaveTextContent('The server did not answer.');
  });

  it('removes an archived message at once and brings it back if the move fails', async () => {
    const { user, api } = renderApp();
    await rows();
    api.failNext('moveMessages', { kind: 'storage', message: 'Disk full.' });
    await user.click(await within(reader()).findByRole('button', { name: 'Archive' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not archive');
    expect((await rows()).getAllByRole('button')).toHaveLength(6);

    await user.click(await within(reader()).findByRole('button', { name: 'Archive' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(5);
    });
    expect(within(reader()).getByRole('heading', { level: 2 })).toHaveTextContent('Nightly build passed on main');
  });

  it('asks before deleting forever from Trash', async () => {
    const { user, api } = renderApp();
    await rows();
    await api.moveMessages(['m6'], { role: 'trash' });
    await user.click(await accountFolders('Work').findByRole('button', { name: 'Trash' }));
    await user.click(await within(reader()).findByRole('button', { name: 'Delete forever' }));
    const dialog = screen.getByRole('dialog', { name: 'Delete forever?' });
    await user.click(within(dialog).getByRole('button', { name: 'Delete forever' }));
    expect(await screen.findByText('Trash is empty')).toBeInTheDocument();
  });
});

describe('sync status and notifications', () => {
  it('shows sync problems in the top bar and status bar', async () => {
    const { api } = renderApp();
    await rows();
    act(() => {
      api.emit('mail://sync-status', { accountId: 'work', state: 'error', lastSyncedAt: MOCK_NOW, message: 'Login failed' });
    });
    expect(screen.getByText('Sync problem')).toBeInTheDocument();
    expect(screen.getByText('Could not sync you@work.example: Login failed')).toBeInTheDocument();
  });

  it('checks for new mail on request', async () => {
    const { user, api } = renderApp();
    await rows();
    const spy = vi.spyOn(api, 'syncNow');
    await user.click(screen.getByRole('button', { name: 'Check for new mail' }));
    expect(spy).toHaveBeenCalledWith(null);
    expect(await screen.findByText(/Last synced/)).toBeInTheDocument();
  });

  it('refreshes counts on new mail and leaves desktop notifications to the backend', async () => {
    const created = vi.fn();
    vi.stubGlobal('Notification', created);
    try {
      const { api } = renderApp();
      await rows();
      await api.setFlags(['m6'], { seen: false });
      act(() => {
        api.emit('mail://new-mail', { accountId: 'work', count: 1 });
      });
      expect(await unified().findByRole('button', { name: 'Inbox 5 unread' })).toBeInTheDocument();
      expect(created).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('plays the new-mail chime only when notifications and sound are on', async () => {
    const chime = vi.spyOn(chimeModule, 'playChime').mockReturnValue(true);
    const { api } = renderApp();
    await rows();
    act(() => {
      api.emit('mail://new-mail', { accountId: 'work', count: 1 });
    });
    expect(chime).toHaveBeenCalledTimes(1);

    cleanup();
    chime.mockClear();
    const quiet = renderApp({ settings: { theme: 'system', remoteImages: 'block', notifications: true, sound: false } });
    await rows();
    act(() => {
      quiet.api.emit('mail://new-mail', { accountId: 'work', count: 1 });
    });
    expect(chime).not.toHaveBeenCalled();
  });

  it('refetches accounts when the backend says they changed', async () => {
    const { api } = renderApp();
    await rows();
    await api.updateAccount('personal', { displayName: 'Home' });
    await vi.waitFor(() => {
      expect(accountToggle('Home')).toBeInTheDocument();
    });
  });
});

describe('first run', () => {
  it('shows the welcome screen when there are no accounts', async () => {
    renderApp({ accounts: [] });
    expect(await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' })).toBeInTheDocument();
    expect(screen.getByRole('form', { name: 'Add an email account' })).toBeInTheDocument();
    expect(MOCK_ACCOUNTS).toHaveLength(2);
  });
});
