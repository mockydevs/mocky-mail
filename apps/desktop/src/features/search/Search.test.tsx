import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { accountFolders, renderApp, rows } from '../../test/renderApp';

const box = () => screen.getByRole('searchbox', { name: 'Search mail' });
const strip = () => within(screen.getByRole('region', { name: 'Search options' }));

describe('search', () => {
  it('shows the search as chips with what it did not understand, and a chip’s × takes the term out', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(box());
    await user.paste('from:wanjiru is:bogus');
    const terms = within(await screen.findByRole('list', { name: 'Search terms' }));
    expect(terms.getAllByRole('listitem').map((li) => li.textContent)).toEqual(['from: wanjiru']);
    expect(strip().getByText('is: can be unread, read, flagged, pinned or answered')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(1);
    });

    await user.click(terms.getByRole('button', { name: 'Remove from: wanjiru' }));
    expect(box()).toHaveValue('is:bogus');
    await user.clear(box());
    await user.click(box());
    await user.paste('-is:read "nightly build"');
    const next = within(await screen.findByRole('list', { name: 'Search terms' }));
    expect(await next.findByText('not is: read')).toBeInTheDocument();
    expect(next.getByText('“nightly build”')).toBeInTheDocument();
    await user.click(next.getByRole('button', { name: 'Remove not is: read' }));
    expect(box()).toHaveValue('"nightly build"');
  });

  it('shows "or" between terms joined by OR, and removing one side takes the OR with it', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(box());
    await user.paste('from:wanjiru OR from:peter');
    const terms = within(await screen.findByRole('list', { name: 'Search terms' }));
    const items = await terms.findAllByRole('listitem');
    expect(items.map((li) => li.textContent)).toEqual(['from: wanjiru or', 'from: peter']);
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(2);
    });
    await user.click(terms.getByRole('button', { name: 'Remove from: peter' }));
    expect(box()).toHaveValue('from:wanjiru');
  });

  it('inserts an example from the search tips', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Search tips' }));
    const tips = within(screen.getByRole('dialog', { name: 'Search tips' }));
    expect(tips.getByText('With an attachment')).toBeInTheDocument();
    await user.click(tips.getByRole('button', { name: 'Add has:attachment' }));
    expect(screen.queryByRole('dialog', { name: 'Search tips' })).not.toBeInTheDocument();
    expect(box()).toHaveValue('has:attachment ');
    expect(box()).toHaveFocus();
    // Three inbox messages have attachments (Wanjiru, Peter, Grace).
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(3);
    });
    // A second example goes after the first; Esc closes the tips without clearing.
    await user.click(screen.getByRole('button', { name: 'Search tips' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Search tips' })).getByRole('button', { name: 'Add is:unread' }));
    expect(box()).toHaveValue('has:attachment is:unread ');
    await user.click(screen.getByRole('button', { name: 'Search tips' }));
    await user.keyboard('{Tab}{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Search tips' })).not.toBeInTheDocument();
    expect(box()).toHaveValue('has:attachment is:unread ');
  });

  it('suggests values for the operator being typed, picked with the arrows and Enter or a click', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(box());
    await user.paste('category:');
    const list = within(screen.getByRole('listbox', { name: 'Suggestions for category:' }));
    expect(list.getAllByRole('option').map((o) => o.textContent)).toEqual(['Clients', 'Invoices', 'Family']);
    await user.keyboard('{ArrowDown}{ArrowDown}');
    expect(list.getByRole('option', { name: 'Invoices' })).toHaveAttribute('aria-selected', 'true');
    expect(box()).toHaveAttribute('aria-activedescendant', list.getByRole('option', { name: 'Invoices' }).id);
    await user.keyboard('{Enter}');
    expect(box()).toHaveValue('category:Invoices ');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();

    await user.paste('is:un');
    await user.click(within(screen.getByRole('listbox', { name: 'Suggestions for is:' })).getByRole('option', { name: /unread/ }));
    expect(box()).toHaveValue('category:Invoices is:unread ');

    // People after from:, from the address book of senders and recipients.
    await user.paste('from:wan');
    const people = within(await screen.findByRole('listbox', { name: 'Suggestions for from:' }));
    await user.click(people.getByRole('option', { name: /Wanjiru Kamau/ }));
    expect(box()).toHaveValue('category:Invoices is:unread from:wanjiru@fitout.example ');

    // Folders after in:; Esc closes the list first, then clears the search.
    await user.paste('in:arc');
    expect(within(screen.getByRole('listbox', { name: 'Suggestions for in:' })).getByRole('option', { name: /archive/ })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    expect(box()).toHaveValue('category:Invoices is:unread from:wanjiru@fitout.example in:arc');
    await user.keyboard('{Escape}');
    expect(box()).toHaveValue('');
  });

  it('searches the folder, its account or everything, shows where results are, and remembers the choice', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Inbox/ }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(4);
    });
    const spy = vi.spyOn(api, 'listMessages');
    await user.click(box());
    await user.paste('INV-0142');
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(1);
    });
    expect(spy.mock.calls.at(-1)?.[0]).toMatchObject({ search: 'INV-0142', searchScope: 'folder', folderId: 'work:inbox' });
    const scope = strip().getByRole('combobox', { name: 'Search in' });
    expect([...scope.querySelectorAll('option')].map((o) => o.textContent)).toEqual(['This folder', 'This account', 'All accounts']);

    await user.selectOptions(scope, 'This account');
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(2);
    });
    expect(spy.mock.calls.at(-1)?.[0]).toMatchObject({ searchScope: 'account' });
    const list = await rows();
    expect(list.getByText('In Sent')).toBeInTheDocument();
    expect(list.getByText('In Inbox')).toBeInTheDocument();

    // Cleared and searched again: still the whole account.
    await user.clear(box());
    expect(screen.queryByRole('region', { name: 'Search options' })).not.toBeInTheDocument();
    await user.click(box());
    await user.paste('logo');
    expect(strip().getByRole('combobox', { name: 'Search in' })).toHaveValue('account');
    await vi.waitFor(async () => {
      expect((await rows()).getByText('In Clients')).toBeInTheDocument();
    });
  });

  it('offers the folders below a folder, and every folder from a unified view', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(box());
    await user.paste('lunch');
    const scope = strip().getByRole('combobox', { name: 'Search in' });
    expect([...scope.querySelectorAll('option')].map((o) => o.textContent)).toEqual([
      'Inbox of every account',
      'Every folder of every account',
    ]);
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(1);
    });
    await user.selectOptions(scope, 'Every folder of every account');
    // The draft reply is found too, and says it is in Drafts.
    expect(await (await rows()).findByText('In Drafts')).toBeInTheDocument();

    // A folder with folders inside it can include them.
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Clients/ }));
    expect([...strip().getByRole('combobox', { name: 'Search in' }).querySelectorAll('option')].map((o) => o.textContent)).toEqual([
      'This folder',
      'Including subfolders',
      'This account',
      'All accounts',
    ]);
    expect(strip().getByRole('combobox', { name: 'Search in' })).toHaveValue('all');
  });
});
