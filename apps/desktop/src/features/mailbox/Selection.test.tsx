import { act, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

import { accountFolders, reader, renderApp, rows, unified } from '../../test/renderApp';
import { rangeOf } from './useSelection';

const row = async (name: RegExp) => (await rows()).getByRole('button', { name });
const bar = () => within(screen.getByRole('toolbar', { name: 'Selected messages' }));
const checkedIds = () =>
  [...document.querySelectorAll<HTMLInputElement>('.row__check')]
    .filter((box) => box.checked)
    .map((box) => box.closest('[data-row-id]')?.getAttribute('data-row-id'));

describe('multi-select', () => {
  it('ticks rows with Ctrl-click (the open one too), extends with Shift-click and clears with Esc', async () => {
    const { user } = renderApp();
    await user.click(await row(/Brian Otieno/));
    fireEvent.click(await row(/Calendar/), { ctrlKey: true });
    expect(screen.getByText('2 selected')).toBeInTheDocument();
    expect(checkedIds()).toEqual(['m3', 'm5']);

    // Shift-click: from the last row picked to this one.
    fireEvent.click(await row(/Grace Njeri/), { shiftKey: true });
    expect(checkedIds()).toEqual(['m3', 'm5', 'm6', 'm7']);

    // Cmd-click unticks.
    fireEvent.click(await row(/Peter Mwangi/), { metaKey: true });
    expect(screen.getByText('3 selected')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('toolbar', { name: 'Selected messages' })).not.toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 1, name: 'Inbox' })).toBeInTheDocument();
  });

  it('ticks with the row checkbox and Ctrl+A, and offers to select all', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server: Nightly build/ }));
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Select all 6' }));
    expect(screen.getByText('6 selected')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await user.keyboard('{Control>}a{/Control}');
    expect(screen.getByText('6 selected')).toBeInTheDocument();
  });

  it('marks every ticked conversation read, then unread, at once', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Wanjiru Kamau/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(bar().getByRole('button', { name: 'Mark as read' }));
    expect(await unified().findByRole('button', { name: 'Inbox 2 unread' })).toBeInTheDocument();
    expect(bar().getByRole('button', { name: 'Mark as unread' })).toBeInTheDocument();
    await user.click(bar().getByRole('button', { name: 'Mark as unread' }));
    expect(await unified().findByRole('button', { name: 'Inbox 4 unread' })).toBeInTheDocument();
  });

  it('flags, pins and archives several messages, with Undo', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Peter Mwangi/ }));
    await user.click(bar().getByRole('button', { name: 'Flag' }));
    await user.click(bar().getByRole('button', { name: 'Pin' }));
    await vi.waitFor(async () => {
      const listed = await api.listMessages({ accountId: 'work', folderId: 'work:inbox', role: null, filter: 'pinned', search: null, offset: 0, limit: 50 });
      expect(listed.items.map((m) => [m.id, m.flagged, m.pinned])).toEqual([
        ['m2', true, true],
        ['m6', true, true],
      ]);
    });

    await user.click(bar().getByRole('button', { name: 'Archive' }));
    expect(await screen.findByText('Archived')).toBeInTheDocument();
    expect(screen.getByText('2 messages')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(4);
    });
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(6);
    });
  });

  it('asks once, with the count, before deleting several messages forever from Trash', async () => {
    const { user, api } = renderApp();
    await rows();
    await api.moveMessages(['m2', 'm6'], { role: 'trash' });
    await user.click(await accountFolders('Work').findByRole('button', { name: 'Trash' }));
    await rows('Trash');
    await user.keyboard('{Control>}a{/Control}');
    await user.click(bar().getByRole('button', { name: 'Delete forever' }));
    const dialog = screen.getByRole('dialog', { name: 'Delete 2 messages forever?' });
    expect(dialog).toHaveTextContent('These 2 messages are removed from the server');
    await user.click(within(dialog).getByRole('button', { name: 'Delete forever' }));
    expect(await screen.findByText('Trash is empty')).toBeInTheDocument();
  });

  it('marks several as junk, and back as not junk from Spam', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Calendar/ }));
    await user.click(bar().getByRole('button', { name: 'Junk' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(4);
    });
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Spam/ }));
    await rows('Spam');
    await user.keyboard('{Control>}a{/Control}');
    await user.click(bar().getByRole('button', { name: 'Not junk' }));
    expect(await screen.findByText('No spam')).toBeInTheDocument();
  });

  it('acts on the ticked rows with the keyboard shortcuts too', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Calendar/ }));
    await user.keyboard('e');
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(4);
    });
  });
});

describe('rangeOf', () => {
  const items = ['a', 'b', 'c', 'd'].map((id) => ({ id }) as never);
  it('runs either way and falls back to the row alone', () => {
    expect(rangeOf(items, 'b', 'd')).toEqual(['b', 'c', 'd']);
    expect(rangeOf(items, 'd', 'b')).toEqual(['b', 'c', 'd']);
    expect(rangeOf(items, null, 'c')).toEqual(['c']);
    expect(rangeOf(items, 'gone', 'c')).toEqual(['c']);
    expect(rangeOf(items, 'a', 'gone')).toEqual([]);
  });
});

describe('selecting on a phone', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('starts with a long press, then taps add and remove rows', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query.startsWith('(max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    const { user } = renderApp();
    const brian = await row(/Brian Otieno/);
    // Android reports a long press as a context menu.
    fireEvent.contextMenu(brian);
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    await user.click(await row(/Grace Njeri/));
    expect(screen.getByText('2 selected')).toBeInTheDocument();
    await user.click(await row(/Brian Otieno/));
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    // Taps while selecting don't open anything.
    expect(document.querySelector('.app--reading')).toBeNull();
    await user.click(bar().getByRole('button', { name: 'Archive' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(5);
    });
    expect(screen.queryByText('selected', { exact: false })).not.toBeInTheDocument();
  });

  it('also starts from a held finger, without a context menu event', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query.startsWith('(max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    renderApp();
    const brian = await row(/Brian Otieno/);
    vi.useFakeTimers();
    try {
      fireEvent.pointerDown(brian, { button: 0 });
      act(() => {
        vi.advanceTimersByTime(600);
      });
      fireEvent.pointerUp(brian);
      fireEvent.click(brian);
    } finally {
      vi.useRealTimers();
    }
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    expect(within(reader()).queryByRole('heading', { name: 'Lunch on Friday?' })).not.toBeInTheDocument();
  });
});
