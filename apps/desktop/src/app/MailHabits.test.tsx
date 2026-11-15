import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_SETTINGS } from '../api/mock/mockData';
import { reader, renderApp, rows, unified } from '../test/renderApp';

const composeForm = () => within(screen.getByRole('form', { name: 'New message' }));

describe('undo', () => {
  it('undoes an archive from the toast', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Archive' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(5);
    });
    const toast = screen.getByText('Archived').closest('.toast');
    if (!(toast instanceof HTMLElement)) throw new Error('no toast');
    await user.click(within(toast).getByRole('button', { name: 'Undo' }));
    await vi.waitFor(async () => {
      expect((await rows()).getAllByRole('button')).toHaveLength(6);
    });
  });

  it('holds a sent message for the undo window and brings it back to compose on Undo', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    await user.type(composeForm().getByLabelText('To'), 'peter@client.example');
    await user.type(composeForm().getByLabelText('Subject'), 'Second thoughts');
    await user.click(composeForm().getByRole('button', { name: 'Send' }));

    expect(await screen.findByText('You have 5 seconds to undo.')).toBeInTheDocument();
    const queued = (await api.listOutbox()).at(-1);
    expect(queued?.status).toMatchObject({ state: 'queued' });

    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(await composeForm().findByDisplayValue('Second thoughts')).toBeInTheDocument();
    expect((await api.listOutbox()).some((e) => e.subject === 'Second thoughts')).toBe(false);
  });

  it('sends at once when undo send is off', async () => {
    const { user } = renderApp({ settings: { ...MOCK_SETTINGS, undoSendSecs: 0 } });
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    await user.type(composeForm().getByLabelText('To'), 'peter@client.example');
    await user.click(composeForm().getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('Your message is on its way.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });
});

describe('mark as read', () => {
  it('waits a few seconds before marking read, so a quick peek stays unread', async () => {
    const { user } = renderApp({ settings: { ...MOCK_SETTINGS, markRead: 'delay' } });
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    expect(unified().getByRole('button', { name: 'Inbox 4 unread' })).toBeInTheDocument();
    expect(await unified().findByRole('button', { name: 'Inbox 3 unread' }, { timeout: 4500 })).toBeInTheDocument();
  });

  it('leaves mail unread when set to manual', async () => {
    const { user } = renderApp({ settings: { ...MOCK_SETTINGS, markRead: 'manual' } });
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 3300));
    expect(unified().getByRole('button', { name: 'Inbox 4 unread' })).toBeInTheDocument();
  });
});

describe('message list', () => {
  it('groups messages under day headings', async () => {
    renderApp();
    await rows();
    const headings = screen
      .getByRole('list', { name: /messages$/ })
      .querySelectorAll('h2');
    const labels = [...headings].map((h) => h.textContent);
    expect(labels[0]).toBe('Today');
    expect(labels.length).toBeGreaterThan(1);
    // Each heading appears once, in order.
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe('recipient suggestions', () => {
  it('suggests people from past mail while typing and adds the chosen one as a chip', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const to = composeForm().getByLabelText('To');
    await user.type(to, 'wan');
    const option = await screen.findByRole('option', { name: /Wanjiru Kamau/ });
    expect(to).toHaveAttribute('aria-expanded', 'true');

    await user.keyboard('{ArrowDown}');
    expect(option).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{Enter}');
    expect(composeForm().getByRole('list', { name: 'To recipients' })).toHaveTextContent('Wanjiru Kamau');
    expect(to).toHaveValue('');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });
});
