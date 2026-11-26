import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { reader, renderApp, rows } from '../../test/renderApp';
import { cleanHex } from './CategoryEditor';

async function openSettings(user: ReturnType<typeof renderApp>['user']) {
  await rows();
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  return within(screen.getByRole('region', { name: 'Categories' }));
}

describe('categories in Settings', () => {
  it('adds one with a palette colour or a typed hex', async () => {
    const { user, api } = renderApp();
    const section = await openSettings(user);
    expect(section.getAllByRole('listitem').map((li) => li.querySelector('.chip')?.textContent)).toEqual([
      'Clients',
      'Invoices',
      'Family',
    ]);
    await user.click(section.getByRole('button', { name: 'New category' }));
    const dialog = within(screen.getByRole('dialog', { name: 'New category' }));
    await user.type(dialog.getByRole('textbox', { name: 'Name' }), 'Urgent');
    await user.click(dialog.getByRole('radio', { name: 'Red' }));
    await user.click(dialog.getByRole('button', { name: 'Create' }));
    expect(await section.findByText('Category "Urgent" added.')).toBeInTheDocument();
    expect(await api.listCategories()).toContainEqual({ name: 'Urgent', color: '#b42318' });

    await user.click(section.getByRole('button', { name: 'New category' }));
    const second = within(screen.getByRole('dialog', { name: 'New category' }));
    await user.type(second.getByRole('textbox', { name: 'Name' }), 'Travel');
    const hex = second.getByRole('textbox', { name: 'Or any colour (hex)' });
    await user.clear(hex);
    await user.type(hex, '12AB34');
    await user.click(second.getByRole('button', { name: 'Create' }));
    await vi.waitFor(async () => {
      expect(await api.listCategories()).toContainEqual({ name: 'Travel', color: '#12ab34' });
    });
  });

  it('refuses a name that is taken', async () => {
    const { user } = renderApp();
    const section = await openSettings(user);
    await user.click(section.getByRole('button', { name: 'New category' }));
    const dialog = within(screen.getByRole('dialog', { name: 'New category' }));
    await user.type(dialog.getByRole('textbox', { name: 'Name' }), 'clients');
    await user.click(dialog.getByRole('button', { name: 'Create' }));
    expect(await dialog.findByRole('alert')).toHaveTextContent("There's already a category called clients.");
  });

  it('renames and recolours one on every message, and deletes one', async () => {
    const { user, api } = renderApp();
    const section = await openSettings(user);
    await user.click(section.getByRole('button', { name: 'Edit Clients' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Edit Clients' }));
    const name = dialog.getByRole('textbox', { name: 'Name' });
    await user.clear(name);
    await user.type(name, 'Customers');
    await user.click(dialog.getByRole('radio', { name: 'Teal' }));
    await user.click(dialog.getByRole('button', { name: 'Save' }));
    await section.findByText('Customers');
    const m1 = await api.getMessage('m1', false);
    expect(m1.categories).toEqual(['Customers']);
    expect(await api.listCategories()).toContainEqual({ name: 'Customers', color: '#0f7b6c' });

    await user.click(section.getByRole('button', { name: 'Delete Invoices' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Delete Invoices?' })).getByRole('button', { name: 'Delete category' }));
    await vi.waitFor(() => {
      expect(section.queryByText('Invoices')).not.toBeInTheDocument();
    });
    expect((await api.getMessage('m6', false)).categories).toEqual([]);
  });

  it('reads a hex colour with or without the #', () => {
    expect(cleanHex('2450C8')).toBe('#2450c8');
    expect(cleanHex(' #abcdef ')).toBe('#abcdef');
    expect(cleanHex('#abc')).toBeNull();
  });
});

describe('categorising messages', () => {
  it('ticks categories on the open message and shows them as chips', async () => {
    const { user, api } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const pane = within(reader());
    await pane.findByRole('heading', { name: 'Nightly build passed on main' });
    await user.click(pane.getByRole('button', { name: 'Categorise' }));
    const menu = within(screen.getByRole('menu', { name: 'Categorise' }));
    expect(menu.getByRole('menuitemcheckbox', { name: 'Invoices' })).toHaveAttribute('aria-checked', 'false');
    await user.click(menu.getByRole('menuitemcheckbox', { name: 'Invoices' }));
    // The menu stays open to tick more.
    expect(menu.getByRole('menuitemcheckbox', { name: 'Invoices' })).toHaveAttribute('aria-checked', 'true');
    expect(within(pane.getByRole('list', { name: 'Categories' })).getByText('Invoices')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).categories).toEqual(['Invoices']);
    });
    await user.click(menu.getByRole('menuitemcheckbox', { name: 'Invoices' }));
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).categories).toEqual([]);
    });
  });

  it('makes a new category from the menu and puts it on the selected messages', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Calendar/ }));
    const bar = within(screen.getByRole('toolbar', { name: 'Selected messages' }));
    await user.click(bar.getByRole('button', { name: 'Categorise' }));
    await user.click(screen.getByRole('menuitem', { name: 'New category…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'New category' }));
    await user.type(dialog.getByRole('textbox', { name: 'Name' }), 'Team');
    await user.click(dialog.getByRole('button', { name: 'Create' }));
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).categories).toEqual(['Team']);
      expect((await api.getMessage('m5', false)).categories).toEqual(['Team']);
    });
    expect((await rows()).getByRole('button', { name: /Build server/ })).toHaveTextContent('Team');
  });

  it('shows a mixed tick when only some of the selected messages have a category', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Wanjiru Kamau/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(within(screen.getByRole('toolbar', { name: 'Selected messages' })).getByRole('button', { name: 'Categorise' }));
    expect(screen.getByRole('menuitemcheckbox', { name: 'Clients' })).toHaveAttribute('aria-checked', 'mixed');
  });
});
