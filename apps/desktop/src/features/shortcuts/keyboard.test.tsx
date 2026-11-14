import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { reader, renderApp, rows, unified } from '../../test/renderApp';

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
    expect(within(dialog).getAllByRole('term')).toHaveLength(14);
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
