import { cleanup, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { accountFolders, accountToggle, renderApp, rows } from '../../test/renderApp';
import { COLLAPSED_STORAGE_KEY } from './folderTree';

async function work() {
  await accountFolders('Work').findByRole('button', { name: 'Clients' });
  return accountFolders('Work');
}

describe('folder tree', () => {
  it('nests user folders under their parent and shows a missing parent as a plain label', async () => {
    renderApp();
    await rows();
    const tree = await work();
    const clients = within(tree.getByRole('list', { name: 'Clients folders' }));
    expect(clients.getByRole('button', { name: 'Acme' })).toBeInTheDocument();
    expect(tree.queryByRole('button', { name: 'Projects' })).not.toBeInTheDocument();
    expect(tree.getByText('Projects')).toBeInTheDocument();
    const projects = within(tree.getByRole('list', { name: 'Projects folders' }));
    expect(projects.getByRole('button', { name: '2026' })).toBeInTheDocument();
    expect(tree.getByRole('button', { name: 'Résumés' })).toBeInTheDocument();
  });

  it('nests on the account delimiter, including "."', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(accountToggle('Personal'));
    await accountFolders('Personal').findByRole('button', { name: 'Travel' });
    const travel = within(accountFolders('Personal').getByRole('list', { name: 'Travel folders' }));
    expect(travel.getByRole('button', { name: 'Kenya' })).toBeInTheDocument();
  });

  it('opens a nested folder', async () => {
    const { user, api } = renderApp();
    await rows();
    await api.moveMessages(['m4'], { folderId: 'work:Clients/Acme' });
    await user.click((await work()).getByRole('button', { name: 'Acme' }));
    expect((await rows('Acme')).getAllByRole('button')).toHaveLength(1);
  });

  it('collapses and expands, and remembers it on this computer', async () => {
    const { user } = renderApp();
    await rows();
    const toggle = (await work()).getByRole('button', { name: 'Collapse Clients' });
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await user.click(toggle);
    expect((await work()).queryByRole('button', { name: 'Acme' })).not.toBeInTheDocument();
    expect((await work()).getByRole('button', { name: 'Expand Clients' })).toHaveAttribute('aria-expanded', 'false');
    expect(JSON.parse(window.localStorage.getItem(COLLAPSED_STORAGE_KEY) ?? '[]')).toEqual(['work:Clients']);

    cleanup();
    const second = renderApp();
    await rows();
    expect((await work()).queryByRole('button', { name: 'Acme' })).not.toBeInTheDocument();
    await second.user.click((await work()).getByRole('button', { name: 'Expand Clients' }));
    expect((await work()).getByRole('button', { name: 'Acme' })).toBeInTheDocument();
    expect(JSON.parse(window.localStorage.getItem(COLLAPSED_STORAGE_KEY) ?? '[]')).toEqual([]);
  });

  it('still collapses when storage is blocked', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    const { user } = renderApp();
    await rows();
    await user.click((await work()).getByRole('button', { name: 'Collapse Clients' }));
    expect((await work()).queryByRole('button', { name: 'Acme' })).not.toBeInTheDocument();
  });

  it('is keyboard operable', async () => {
    const { user } = renderApp();
    await rows();
    const toggle = (await work()).getByRole('button', { name: 'Collapse Clients' });
    toggle.focus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('button', { name: 'Expand Clients' })).toHaveFocus();
  });
});
