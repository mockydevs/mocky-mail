import { act, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { deferred, reader, renderApp, rows, unified } from '../../test/renderApp';

const current = async () => (await rows()).getByRole('button', { current: true });

describe('keyboard shortcuts', () => {
  it('moves with j and k and marks opened messages read', async () => {
    const { user } = renderApp();
    await rows();
    await user.keyboard('j');
    expect(await current()).toHaveTextContent('Build server');
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
    await user.keyboard('jk');
    expect(await current()).toHaveTextContent('Build server');
  });

  it('flags with s, toggles read with u, archives with e and deletes with #', async () => {
    const { user } = renderApp();
    await rows();
    await within(reader()).findByRole('heading', { name: /Westlands/ });
    await user.keyboard('s');
    expect(await within(reader()).findByRole('button', { name: 'Flag', pressed: true })).toBeInTheDocument();
    await user.keyboard('u');
    expect(await within(reader()).findByRole('button', { name: 'Mark as unread' })).toBeInTheDocument();
    await user.keyboard('e');
    await vi.waitFor(async () => {
      expect((await rows()).queryByRole('button', { name: /Wanjiru/ })).not.toBeInTheDocument();
    });
    await user.keyboard('#');
    await vi.waitFor(async () => {
      expect((await rows()).queryByRole('button', { name: /Build server/ })).not.toBeInTheDocument();
    });
  });

  it('replies with r, replies all with a, forwards with f and composes with c', async () => {
    const { user } = renderApp();
    await rows();
    await within(reader()).findByRole('heading', { name: /Westlands/ });
    await user.keyboard('r');
    expect(screen.getByRole('form', { name: 'Reply' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await user.keyboard('a');
    const cc = within(within(screen.getByRole('form', { name: 'Reply all' })).getByRole('list', { name: 'Cc recipients' }));
    expect(cc.getByRole('button', { name: 'Remove Amina Hassan <amina@studio.example>' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await user.keyboard('f');
    expect(screen.getByRole('form', { name: 'Forward' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await user.keyboard('c');
    expect(screen.getByRole('form', { name: 'New message' })).toBeInTheDocument();
  });

  it('replies to the message on screen after moving past a slow one', async () => {
    const { user, api } = renderApp();
    await rows();
    await within(reader()).findByRole('heading', { name: /Westlands/ });
    // The build mail (second row) is slow to open; the user is already on the third.
    const slow = deferred();
    const getMessage = api.getMessage.bind(api);
    vi.spyOn(api, 'getMessage').mockImplementation(async (id, allowRemote) => {
      if (id === 'm2') await slow.promise;
      return getMessage(id, allowRemote);
    });
    await user.keyboard('jj');
    await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' });
    await act(async () => {
      slow.resolve();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    await user.keyboard('r');
    const reply = within(screen.getByRole('form', { name: 'Reply' }));
    expect(reply.getByLabelText('Subject')).toHaveValue('Re: Lunch on Friday?');
  });

  it('leaves the mail alone while a dialog is open over it', async () => {
    const { user } = renderApp();
    await rows();
    await within(reader()).findAllByRole('button', { name: /^Preview / });
    await user.click(within(reader()).getByRole('button', { name: 'More actions' }));
    await user.click(within(reader()).getByRole('button', { name: 'View source' }));
    const dialog = await screen.findByRole('dialog', { name: 'Message source' });

    // Archive, delete, flag, mark unread, new message, next.
    await user.keyboard('e#sucj');
    expect(dialog).toBeInTheDocument();
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Close' }));
    expect(await current()).toHaveTextContent('Wanjiru');
    expect(within(reader()).getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'false');
    expect((await rows()).getAllByRole('button')).toHaveLength(6);
  });

  it('does not close a screen with Esc while typing in one of its fields', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Contacts' }));
    const contacts = within(screen.getByRole('region', { name: 'Contacts' }));
    await user.click(contacts.getByRole('button', { name: 'New contact' }));
    const name = contacts.getByRole('textbox', { name: 'Name' });
    await user.type(name, 'Wanjiru Kamau{Escape}');
    expect(name).toHaveValue('Wanjiru Kamau');

    // Away from the fields, Esc closes the screen as before.
    await user.click(contacts.getByRole('button', { name: 'Sync contacts' }));
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('region', { name: 'Contacts' })).not.toBeInTheDocument();
  });

  it('focuses search with / and ignores shortcuts while typing', async () => {
    const { user } = renderApp();
    await rows();
    await user.keyboard('/');
    const search = screen.getByRole('searchbox', { name: 'Search mail' });
    expect(search).toHaveFocus();
    expect(search).toHaveValue('');
    await user.keyboard('jc');
    expect(search).toHaveValue('jc');
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('shows the help sheet with ? and closes it with Esc', async () => {
    const { user } = renderApp();
    await rows();
    await user.keyboard('?');
    const dialog = screen.getByRole('dialog', { name: 'Keyboard shortcuts' });
    expect(within(dialog).getByText('Reply all')).toBeInTheDocument();
    expect(within(dialog).getAllByRole('term')).toHaveLength(18);
    expect(within(dialog).getByText('Move to a folder')).toBeInTheDocument();
    expect(within(dialog).getByText('Undo')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('focuses the reading pane with Enter and closes settings with Esc', async () => {
    const { user } = renderApp();
    await rows();
    await within(reader()).findByRole('heading', { name: /Westlands/ });
    await user.keyboard('{Enter}');
    expect(reader()).toHaveFocus();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.keyboard('{Escape}');
    expect(await rows()).toBeDefined();
  });
});
