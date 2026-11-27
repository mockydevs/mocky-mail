import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { reader, renderApp, rows } from '../../test/renderApp';

/** Pretend the window is phone-sized. */
function phoneWidth() {
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: query.startsWith('(max-width'),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openBuild(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await ctx.user.click((await rows()).getByRole('button', { name: /Build server/ }));
  await within(reader()).findByRole('heading', { name: 'Nightly build passed on main' });
  return ctx;
}

describe('pictures from trusted senders', () => {
  it('always shows pictures from a domain once asked, and stops when asked', async () => {
    const { user, api } = await openBuild();
    const pane = within(reader());
    expect(pane.getByRole('button', { name: 'Show images (1 blocked)' })).toBeInTheDocument();
    expect(pane.getByRole('button', { name: 'Always show from builds@ci.example' })).toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'Always show from ci.example' }));

    expect(await pane.findByText(/Pictures shown because you trust everyone at ci\.example/)).toBeInTheDocument();
    expect(pane.queryByRole('button', { name: /Show images/ })).not.toBeInTheDocument();
    expect(await api.listImageSenders()).toEqual([{ sender: 'ci.example', wholeDomain: true }]);
    const frame = pane.getByTitle('Message: Nightly build passed on main');
    expect(frame.getAttribute('srcdoc')).toContain('badge.svg');

    await user.click(pane.getByRole('button', { name: 'Stop showing pictures from ci.example automatically' }));
    expect(await pane.findByRole('button', { name: 'Show images (1 blocked)' })).toBeInTheDocument();
    expect(pane.queryByText(/Pictures shown because/)).not.toBeInTheDocument();
    expect(await api.listImageSenders()).toEqual([]);
  });

  it('says which sender is trusted, and offers no domain for public mail services', async () => {
    const { user, api } = await openBuild({ imageSenders: [{ sender: 'builds@ci.example', wholeDomain: false }] });
    const pane = within(reader());
    expect(await pane.findByText(/Pictures shown because you trust builds@ci\.example/)).toBeInTheDocument();
    await api.untrustImageSender('builds@ci.example');

    // A sender at a free mail service: only the address can be trusted.
    api.receive({
      ...(await api.getMessage('m2', false)),
      id: 'g1',
      from: { name: 'Friend', address: 'friend@gmail.com' },
      subject: 'Party pictures',
      remoteHtml: '<p>Look</p><img src="https://pics.example/a.jpg" alt="">',
      date: 1_900_000_000,
    });
    await user.click(await (await rows()).findByRole('button', { name: /Party pictures/ }));
    await within(reader()).findByRole('heading', { name: 'Party pictures' });
    expect(within(reader()).getByRole('button', { name: 'Always show from friend@gmail.com' })).toBeInTheDocument();
    expect(within(reader()).queryByRole('button', { name: 'Always show from gmail.com' })).not.toBeInTheDocument();
  });

  it('lists trusted senders in Settings, with Remove', async () => {
    const { user, api } = renderApp({
      imageSenders: [
        { sender: 'ci.example', wholeDomain: true },
        { sender: 'amina@studio.example', wholeDomain: false },
      ],
    });
    await rows();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const list = within(await screen.findByRole('list', { name: 'Trusted senders' }));
    expect(list.getAllByRole('listitem').map((li) => li.textContent)).toEqual([
      'Anyone at ci.exampleRemove',
      'amina@studio.exampleRemove',
    ]);
    await user.click(list.getByRole('button', { name: 'Stop trusting ci.example' }));
    await vi.waitFor(() => {
      expect(list.getAllByRole('listitem')).toHaveLength(1);
    });
    expect(await api.listImageSenders()).toEqual([{ sender: 'amina@studio.example', wholeDomain: false }]);
  });

  it('offers the same on a phone', async () => {
    phoneWidth();
    const { user, api } = await openBuild();
    const pane = within(reader());
    expect(pane.getByRole('button', { name: 'Show images (1 blocked)' })).toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'Always show from builds@ci.example' }));
    expect(await pane.findByText(/Pictures shown because you trust builds@ci\.example/)).toBeInTheDocument();
    expect(await api.listImageSenders()).toEqual([{ sender: 'builds@ci.example', wholeDomain: false }]);
  });
});

describe('search and rules on a phone', () => {
  it('shows the search strip and reorders rules with buttons, not dragging', async () => {
    phoneWidth();
    const { user } = renderApp({
      rules: [
        {
          id: 'a', name: 'First', enabled: true, accountId: 'work', match: 'all', conditions: [], exceptions: [],
          actions: [{ kind: 'flag' }], stopProcessing: false, keepOnServer: false, serverStatus: 'local', serverNote: null, order: 0,
        },
        {
          id: 'b', name: 'Second', enabled: true, accountId: 'work', match: 'all', conditions: [], exceptions: [],
          actions: [{ kind: 'pin' }], stopProcessing: false, keepOnServer: false, serverStatus: 'local', serverNote: null, order: 1,
        },
      ],
    });
    await rows();
    await user.click(screen.getByRole('searchbox', { name: 'Search mail' }));
    await user.paste('is:unread');
    expect(within(screen.getByRole('region', { name: 'Search options' })).getByRole('combobox', { name: 'Search in' })).toBeInTheDocument();
    expect(await screen.findByRole('list', { name: 'Search terms' })).toHaveTextContent('is: unread');

    await user.click(screen.getByRole('button', { name: 'Folders and accounts' }));
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const list = within(await screen.findByRole('list', { name: 'Rules' }));
    expect(list.queryByRole('button', { name: /^Drag/ })).not.toBeInTheDocument();
    await user.click(list.getByRole('button', { name: 'Move Second up' }));
    await vi.waitFor(() => {
      expect(list.getAllByRole('listitem')[0]).toHaveTextContent('Second');
    });
    await user.click(list.getByRole('button', { name: 'Edit Second' }));
    expect(screen.getByRole('dialog', { name: 'Edit rule' })).toBeInTheDocument();
  });
});
