import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { renderApp, rows } from '../../test/renderApp';
import { filterCommands, type PaletteCommand } from './CommandPalette';

const palette = () => within(screen.getByRole('dialog', { name: 'Command palette' }));
const options = () => palette().getAllByRole('option');
const selected = () => palette().getByRole('option', { selected: true });

async function openPalette(setup: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(setup);
  await rows();
  await ctx.user.keyboard('{Control>}k{/Control}');
  return ctx;
}

describe('the command palette', () => {
  it('opens with Ctrl+K (or Cmd+K), with the search box focused, and Esc closes it', async () => {
    const { user } = await openPalette();
    const box = palette().getByRole('combobox', { name: 'Find a command' });
    expect(box).toHaveFocus();
    expect(options().length).toBeGreaterThan(10);
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Command palette' })).not.toBeInTheDocument();
    // Esc closed only the palette, not anything under it.
    expect(await rows()).toBeTruthy();

    await user.keyboard('{Meta>}k{/Meta}');
    expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
  });

  it('opens from the search box too', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('searchbox', { name: 'Search mail' }));
    await user.keyboard('{Control>}k{/Control}');
    expect(palette().getByRole('combobox', { name: 'Find a command' })).toHaveFocus();
  });

  it('filters as you type, moves with the arrow keys and runs with Enter', async () => {
    const { user } = await openPalette();
    await user.keyboard('inbox');
    expect(options()[0]).toHaveTextContent('Inbox');
    expect(options().every((o) => /inbox/i.test(o.textContent))).toBe(true);
    expect(selected()).toBe(options()[0]);
    await user.keyboard('{ArrowDown}');
    expect(selected()).toBe(options()[1]);
    await user.keyboard('{ArrowUp}{ArrowUp}');
    // Wraps round to the last.
    expect(selected()).toBe(options()[options().length - 1]);

    await user.clear(palette().getByRole('combobox'));
    await user.keyboard('clients work');
    // The folder, then the one inside it, shown with its path.
    expect(options()).toHaveLength(2);
    expect(options()[0]).toHaveTextContent(/^ClientsWork/);
    expect(options()[1]).toHaveTextContent(/^Clients › AcmeWork/);
    await user.keyboard('{Enter}');
    expect(screen.queryByRole('dialog', { name: 'Command palette' })).not.toBeInTheDocument();
    expect(await rows('Clients')).toBeTruthy();
  });

  it('says when nothing matches', async () => {
    const { user } = await openPalette();
    await user.keyboard('zzzz');
    expect(palette().queryAllByRole('option')).toHaveLength(0);
    expect(palette().getByRole('status')).toHaveTextContent('Nothing matches “zzzz”');
  });

  it('starts a new message', async () => {
    const { user } = await openPalette();
    await user.keyboard('new message{Enter}');
    expect(await screen.findByRole('form', { name: 'New message' })).toBeInTheDocument();
  });

  it('opens a Settings section, the calendar and the shortcuts', async () => {
    const { user } = await openPalette();
    await user.keyboard('automation{Enter}');
    const settings = within(await screen.findByRole('region', { name: 'Settings' }));
    expect(settings.getByRole('region', { name: 'Automation' })).toHaveFocus();

    await user.keyboard('{Control>}k{/Control}');
    await user.keyboard('calendar{Enter}');
    expect(await screen.findByRole('region', { name: 'November 2026' })).toBeInTheDocument();

    await user.keyboard('{Control>}k{/Control}');
    await user.keyboard('keyboard shortcuts{Enter}');
    const help = within(await screen.findByRole('dialog', { name: 'Keyboard shortcuts' }));
    expect(help.getByText('Ctrl+K')).toBeInTheDocument();
    expect(help.getByText(/Find a command, folder or setting/)).toBeInTheDocument();
  });

  it('toggles the reading pane and Work Offline, and checks for mail', async () => {
    const { user, api } = await openPalette();
    await user.keyboard('turn off the reading pane{Enter}');
    await vi.waitFor(async () => {
      expect((await api.getSettings()).readingPane).toBe('off');
    });

    const sync = vi.spyOn(api, 'syncNow');
    await user.keyboard('{Control>}k{/Control}');
    await user.keyboard('check for new mail{Enter}');
    expect(sync).toHaveBeenCalledWith(null);

    await user.keyboard('{Control>}k{/Control}');
    await user.keyboard('turn on work offline{Enter}');
    await vi.waitFor(async () => {
      expect((await api.getSettings()).workOffline).toBe(true);
    });
    // Offline: nothing to check, and the way back is offered.
    await user.keyboard('{Control>}k{/Control}');
    await user.keyboard('offline');
    expect(palette().getByRole('option', { name: /Turn off Work Offline/ })).toBeInTheDocument();
    expect(palette().queryByRole('option', { name: /Check for new mail/ })).not.toBeInTheDocument();
  });

  it('focuses the search box for Search mail', async () => {
    const { user } = await openPalette();
    await user.keyboard('search mail{Enter}');
    await vi.waitFor(() => {
      expect(screen.getByRole('searchbox', { name: 'Search mail' })).toHaveFocus();
    });
  });

  it('runs a command clicked with the mouse', async () => {
    const { user } = await openPalette();
    await user.keyboard('contacts');
    await user.click(palette().getByRole('option', { name: /^Contacts/ }));
    expect(await screen.findByRole('region', { name: 'Contacts' })).toBeInTheDocument();
  });
});

describe('filterCommands', () => {
  const run = () => undefined;
  const list: PaletteCommand[] = [
    { id: 'a', label: 'Reading pane below the list', group: 'View', run },
    { id: 'b', label: 'Inbox', group: 'Go to', detail: 'Work', run },
    { id: 'c', label: 'Reading settings', group: 'Settings', run },
  ];
  it('needs every word, and puts labels that start with the first word first', () => {
    expect(filterCommands(list, '').map((c) => c.id)).toEqual(['a', 'b', 'c']);
    expect(filterCommands(list, 'reading').map((c) => c.id)).toEqual(['a', 'c']);
    expect(filterCommands(list, 'settings').map((c) => c.id)).toEqual(['c']);
    expect(filterCommands(list, 'inbox work').map((c) => c.id)).toEqual(['b']);
    expect(filterCommands(list, 'inbox home')).toEqual([]);
    expect(filterCommands(list, 'list').map((c) => c.id)).toEqual(['a']);
  });
});
