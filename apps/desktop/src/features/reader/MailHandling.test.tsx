import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { accountFolders, reader, renderApp, rows, unified } from '../../test/renderApp';

async function open(user: ReturnType<typeof renderApp>['user'], name: RegExp, subject: string) {
  await user.click((await rows()).getByRole('button', { name }));
  const pane = within(reader());
  await pane.findByRole('heading', { name: subject });
  return pane;
}

describe('importance', () => {
  it('shows high importance and can be changed from the More menu', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Wanjiru Kamau/, 'Revised quotation for the Westlands office fit-out');
    expect(pane.getByText('High importance')).toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'More actions' }));
    await user.click(pane.getByRole('button', { name: 'Mark as low importance' }));
    expect(await pane.findByText('Low importance')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await api.getMessage('m1', false)).importance).toBe('low');
    });
  });

  it('is sent with a message written with High', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(within(screen.getByRole('navigation', { name: 'Mailboxes' })).getByRole('button', { name: 'New message' }));
    const compose = within(await screen.findByRole('form'));
    const importance = within(compose.getByRole('group', { name: 'Importance' }));
    expect(importance.getByRole('button', { name: 'Normal' })).toHaveAttribute('aria-pressed', 'true');
    await user.click(importance.getByRole('button', { name: /High/ }));
    expect(importance.getByRole('button', { name: /High/ })).toHaveAttribute('aria-pressed', 'true');
    await user.type(compose.getByLabelText('To'), 'amina@studio.example');
    await user.type(compose.getByLabelText('Subject'), 'Today');
    await user.click(compose.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalled();
    });
    expect(send.mock.calls[0]?.[0].importance).toBe('high');
  });
});

describe('flags with due dates', () => {
  it('flags for tomorrow from the flag menu and shows the due date', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'Flag options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Tomorrow' }));
    expect(pane.getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'true');
    expect(pane.getByText('Due tomorrow')).toBeInTheDocument();
    await vi.waitFor(async () => {
      const m2 = await api.getMessage('m2', false);
      expect(m2.flagged).toBe(true);
      expect(m2.flagDue).not.toBeNull();
    });
    expect((await rows()).getByRole('button', { name: /Build server/ })).toHaveTextContent('Due tomorrow');

    await user.click(pane.getByRole('button', { name: 'Flag options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Clear flag' }));
    expect(pane.getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'false');
    expect(pane.queryByText(/^Due/)).not.toBeInTheDocument();
  });

  it('picks a date, and shows a past one as overdue', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'Flag options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Pick a date…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Flag with a due date' }));
    const date = dialog.getByLabelText('Due');
    await user.clear(date);
    await user.type(date, '2026-11-02');
    await user.click(dialog.getByRole('button', { name: 'Flag' }));
    const due = await pane.findByText(/Overdue: due/);
    expect(due.closest('.badge')).toHaveClass('badge--overdue');
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).flagDue).toBe(Math.floor(new Date(2026, 10, 2, 17).getTime() / 1000));
    });
  });

  it('flags without a date', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'Flag options' }));
    await user.click(screen.getByRole('menuitem', { name: 'No date' }));
    await vi.waitFor(async () => {
      const m2 = await api.getMessage('m2', false);
      expect([m2.flagged, m2.flagDue]).toEqual([true, null]);
    });
  });
});

describe('pinning and junk from the reading pane', () => {
  it('pins and unpins the open message', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Peter Mwangi/, 'Invoice INV-0142 paid');
    await user.click(pane.getByRole('button', { name: 'Pin' }));
    expect(pane.getByRole('button', { name: 'Unpin' })).toHaveAttribute('aria-pressed', 'true');
    await vi.waitFor(async () => {
      expect((await api.getMessage('m6', false)).pinned).toBe(true);
    });
    // Pinned messages come first.
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')[0]).toHaveAccessibleName(/Peter Mwangi/);
    });
  });

  it('moves a message to Spam with Junk, and back with Not junk', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'Junk' }));
    expect(await screen.findByText('Marked as junk')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await rows()).queryByRole('button', { name: /Build server/ })).not.toBeInTheDocument();
    });
    await user.click(await accountFolders('Work').findByRole('button', { name: /^Spam/ }));
    await user.click((await rows('Spam')).getByRole('button', { name: /Build server/ }));
    const spam = within(reader());
    await spam.findByRole('heading', { name: 'Nightly build passed on main' });
    expect(spam.queryByRole('button', { name: 'Junk' })).not.toBeInTheDocument();
    await user.click(spam.getByRole('button', { name: 'Not junk' }));
    expect(await screen.findByText('No spam')).toBeInTheDocument();
    expect((await api.getMessage('m2', false)).folderId).toBe('work:inbox');
  });
});

describe('message windows', () => {
  it('opens the message in a window of its own from More, or by double-clicking the row', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'More actions' }));
    await user.click(pane.getByRole('button', { name: 'Open in new window' }));
    await user.dblClick((await rows()).getByRole('button', { name: /Calendar/ }));
    await vi.waitFor(() => {
      expect(api.openedWindows).toEqual(['m2', 'm5']);
    });
  });
});

describe('undo and redo', () => {
  it('takes back a flag with Ctrl+Z and makes it again with Ctrl+Y', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    await user.click(pane.getByRole('button', { name: 'Flag' }));
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).flagged).toBe(true);
    });
    await user.keyboard('{Control>}z{/Control}');
    expect(await screen.findByText('Undone')).toBeInTheDocument();
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).flagged).toBe(false);
    });
    expect(pane.getByRole('button', { name: 'Flag' })).toHaveAttribute('aria-pressed', 'false');
    await user.keyboard('{Control>}y{/Control}');
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).flagged).toBe(true);
    });
    await user.keyboard('{Control>}{Shift>}z{/Shift}{/Control}');
    expect(await screen.findByText('Nothing to redo')).toBeInTheDocument();
  });

  it('brings back an archived message and puts read state back', async () => {
    const { user, api } = renderApp();
    const pane = await open(user, /Build server/, 'Nightly build passed on main');
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'Mark as unread' }));
    expect(await unified().findByRole('button', { name: 'Inbox 4 unread' })).toBeInTheDocument();
    await user.click(within(reader()).getByRole('button', { name: 'Archive' }));
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).folderId).toBe('work:archive');
    });
    await user.keyboard('{Control>}z{/Control}');
    await vi.waitFor(async () => {
      expect((await api.getMessage('m2', false)).folderId).toBe('work:inbox');
    });
    await user.keyboard('{Control>}z{/Control}');
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' })).toBeInTheDocument();
  });

  it('cannot undo deleting forever, so it asks first', async () => {
    const { user, api } = renderApp();
    await rows();
    await api.moveMessages(['m6'], { role: 'trash' });
    await user.click(await accountFolders('Work').findByRole('button', { name: 'Trash' }));
    await user.click(await within(reader()).findByRole('button', { name: 'Delete forever' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Delete forever?' })).getByRole('button', { name: 'Delete forever' }));
    expect(await screen.findByText('Trash is empty')).toBeInTheDocument();
    await user.keyboard('{Control>}z{/Control}');
    expect(await screen.findByText('Nothing to undo')).toBeInTheDocument();
  });

  it('leaves Ctrl+Z to a text field being typed in', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('searchbox', { name: 'Search mail' }));
    await user.keyboard('{Control>}z{/Control}');
    expect(screen.queryByText('Nothing to undo')).not.toBeInTheDocument();
  });
});

