import { createEvent, fireEvent, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { accountFolders, reader, renderApp, rows } from '../../test/renderApp';

const bar = () => within(screen.getByRole('toolbar', { name: 'Selected messages' }));

describe('Move to and Copy to', () => {
  it('moves the open message to a folder picked from the tree, with "v"', async () => {
    const { user, api } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Peter Mwangi/ }));
    await within(reader()).findByRole('heading', { name: 'Invoice INV-0142 paid' });
    await user.keyboard('v');
    const dialog = screen.getByRole('dialog', { name: 'Move this message to…' });
    expect(dialog).toHaveTextContent('Folders in Work.');
    const tree = within(within(dialog).getByRole('navigation', { name: 'Folders' }));
    // The folder it is in already can't be picked; containers are labels only.
    expect(tree.getByRole('button', { name: 'Inbox' })).toBeDisabled();
    expect(tree.queryByRole('button', { name: 'Projects' })).not.toBeInTheDocument();
    expect(tree.getByText('Projects')).toBeInTheDocument();
    await user.click(tree.getByRole('button', { name: 'Acme' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(await screen.findByText('Moved to Acme')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await rows()).queryByRole('button', { name: /Peter Mwangi/ })).not.toBeInTheDocument();
    });
    const listed = await api.listMessages({ accountId: 'work', folderId: 'work:Clients/Acme', role: null, filter: 'all', search: null, offset: 0, limit: 50 });
    expect(listed.items.map((m) => m.id)).toEqual(['m6']);
  });

  it('filters the folders by name', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(bar().getByRole('button', { name: 'Move to…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Move this message to…' }));
    await user.type(dialog.getByRole('searchbox', { name: 'Find a folder' }), 'ac');
    const tree = within(dialog.getByRole('navigation', { name: 'Folders' }));
    expect(tree.getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual(['Clients / Acme']);
    await user.clear(dialog.getByRole('searchbox', { name: 'Find a folder' }));
    await user.type(dialog.getByRole('searchbox', { name: 'Find a folder' }), 'zzz');
    expect(dialog.getByText(/No folder matches/)).toBeInTheDocument();
  });

  it('copies, leaving the original where it is', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(bar().getByRole('button', { name: 'Copy to…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Copy this message to…' }));
    await user.click(within(dialog.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Clients' }));
    expect(await screen.findByText('Copied to Clients')).toBeInTheDocument();
    expect((await rows()).getByRole('button', { name: /Build server/ })).toBeInTheDocument();
    await vi.waitFor(async () => {
      const listed = await api.listMessages({ accountId: 'work', folderId: 'work:Clients', role: null, filter: 'all', search: null, offset: 0, limit: 50 });
      expect(listed.items.map((m) => m.subject)).toContain('Nightly build passed on main');
    });
  });

  it('only offers folders every account has for messages from several accounts', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Brian Otieno/ }));
    await user.click(bar().getByRole('button', { name: 'Move to…' }));
    const dialog = screen.getByRole('dialog', { name: 'Move 2 messages to…' });
    expect(dialog).toHaveTextContent('different accounts');
    const choices = within(within(dialog).getByRole('navigation', { name: 'Folders' })).getAllByRole('button');
    expect(choices.map((b) => b.textContent)).toEqual(['Inbox', 'Archive', 'Trash']);
    await user.click(within(dialog).getByRole('button', { name: 'Archive' }));
    await vi.waitFor(async () => {
      const work = await api.listMessages({ accountId: null, folderId: null, role: 'archive', filter: 'all', search: null, offset: 0, limit: 50 });
      expect(work.items.map((m) => m.id).sort()).toEqual(['m2', 'm3']);
    });
  });
});

/** A drag event with a DataTransfer stand-in (jsdom has none). */
function drag(type: 'dragStart' | 'dragOver' | 'drop' | 'dragEnter', element: Element, dropEffect = 'move') {
  const transfer = { dropEffect, effectAllowed: 'all', setData: vi.fn(), getData: vi.fn() };
  const event = createEvent[type](element, { dataTransfer: transfer });
  fireEvent(element, event);
  return event;
}

describe('dragging messages to the sidebar', () => {
  it('moves a dragged row to a folder of its account, and highlights where it can go', async () => {
    const { api } = renderApp();
    const list = await rows();
    const clients = await accountFolders('Work').findByRole('button', { name: 'Clients' });
    drag('dragStart', list.getByRole('button', { name: /Build server/ }));
    expect(clients).toHaveClass('nav__item--drop');
    drag('dragEnter', clients);
    expect(clients).toHaveClass('nav__item--drop-over');
    const over = drag('dragOver', clients);
    expect(over.defaultPrevented).toBe(true);
    drag('drop', clients);
    expect(await screen.findByText('Moved to Clients')).toBeInTheDocument();
    await vi.waitFor(async () => {
      const listed = await api.listMessages({ accountId: 'work', folderId: 'work:Clients', role: null, filter: 'all', search: null, offset: 0, limit: 50 });
      expect(listed.items.map((m) => m.id).sort()).toEqual(['m2', 'm4']);
    });
    expect(clients).not.toHaveClass('nav__item--drop');
  });

  it('copies with Ctrl held, and drags every ticked row together', async () => {
    const { user, api } = renderApp();
    const list = await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Calendar/ }));
    const archive = await accountFolders('Work').findByRole('button', { name: 'Archive' });
    drag('dragStart', list.getByRole('button', { name: /Calendar/ }));
    drag('drop', archive, 'copy');
    expect(await screen.findByText('Copied to Archive')).toBeInTheDocument();
    await vi.waitFor(async () => {
      const listed = await api.listMessages({ accountId: 'work', folderId: 'work:archive', role: null, filter: 'all', search: null, offset: 0, limit: 50 });
      expect(listed.items.map((m) => m.subject).sort()).toEqual(['Invitation: Sprint review, Fri 14:00', 'Nightly build passed on main']);
    });
    expect(list.getByRole('button', { name: /Calendar/ })).toBeInTheDocument();
  });

  it("doesn't accept messages on another account's folders", async () => {
    const { user } = renderApp({ favorites: ['work:Clients'] });
    await rows();
    await user.click(screen.getByRole('button', { name: /^Personal \(/ }));
    const brian = await (await rows('Inbox')).findByRole('button', { name: /Brian Otieno/ });
    const travel = await accountFolders('Personal').findByRole('button', { name: 'Travel' });
    const workClients = within(screen.getByRole('list', { name: 'Favourites' })).getByRole('button', { name: /^Clients/ });
    drag('dragStart', brian);
    expect(travel).toHaveClass('nav__item--drop');
    expect(workClients).not.toHaveClass('nav__item--drop');
    expect(drag('dragOver', workClients).defaultPrevented).toBe(false);
    expect(drag('dragOver', travel).defaultPrevented).toBe(true);
  });
});
