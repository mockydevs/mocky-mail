import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { vi } from 'vitest';

import { createMockMailApi } from '../api/mockMailApi';
import type { MailApi } from '../api/types';
import { THEME_STORAGE_KEY } from '../theme/theme';
import { App } from './App';

const NOW = () => new Date('2026-11-13T07:15:00Z');

function setup(api: MailApi = createMockMailApi({ now: NOW })) {
  const user = userEvent.setup();
  render(<App api={api} now={NOW} />);
  return { user, api };
}

const folders = () => within(screen.getByRole('list', { name: 'Folders' }));
const rows = async (name = 'Inbox messages') => within(await screen.findByRole('list', { name }));
const reader = () => screen.getByRole('article', { name: 'Reading pane' });

describe('mailbox', () => {
  it('opens the inbox with the newest message shown but not marked read', async () => {
    setup();
    const list = await rows();
    expect(list.getAllByRole('button')).toHaveLength(8);
    expect(
      await within(reader()).findByRole('heading', { name: 'Revised quotation for the Westlands office fit-out' }),
    ).toBeInTheDocument();
    expect(list.getByRole('button', { current: true })).toHaveTextContent('Wanjiru Kamau');
    expect(folders().getByLabelText('4 unread')).toBeInTheDocument();
  });

  it('shows the message the user clicks and marks it read', async () => {
    const { user } = setup();
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByRole('heading', { name: 'Lunch on Friday?' })).toBeInTheDocument();
    expect(within(reader()).getByText(/new place near Yaya Centre/)).toBeInTheDocument();
    expect(await folders().findByLabelText('3 unread')).toBeInTheDocument();
  });

  it('switches folders', async () => {
    const { user } = setup();
    await rows();
    await user.click(folders().getByRole('button', { name: /^Sent/ }));
    expect(screen.getByRole('heading', { level: 1, name: 'Sent' })).toBeInTheDocument();
    const sent = await rows('Sent messages');
    expect(sent.getAllByRole('button')).toHaveLength(2);
    expect(folders().getByRole('button', { name: /^Sent/ })).toHaveAttribute('aria-current', 'page');
  });

  it('shows an empty state for an empty folder and no message selected', async () => {
    const { user } = setup();
    await rows();
    await user.click(folders().getByRole('button', { name: /^Spam/ }));
    expect(await screen.findByText('No spam')).toBeInTheDocument();
    expect(within(reader()).getByText('No message selected')).toBeInTheDocument();
  });

  it('filters by account and toggles back off', async () => {
    const { user } = setup();
    await rows();
    const personal = screen.getByRole('button', { name: /Personal/ });
    await user.click(personal);
    expect(personal).toHaveAttribute('aria-pressed', 'true');
    expect((await rows()).getAllByRole('button')).toHaveLength(2);
    expect(screen.getByText('2 messages, Personal')).toBeInTheDocument();

    await user.click(personal);
    expect(personal).toHaveAttribute('aria-pressed', 'false');
    expect((await rows()).getAllByRole('button')).toHaveLength(8);
  });

  it('filters unread and flagged', async () => {
    const { user } = setup();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Flagged' }));
    const flagged = await rows();
    expect(flagged.getAllByRole('button')).toHaveLength(1);
    expect(flagged.getByRole('button')).toHaveTextContent('Amina Hassan');

    await user.click(screen.getByRole('button', { name: 'Unread' }));
    expect(screen.getByRole('button', { name: 'Unread' })).toHaveAttribute('aria-pressed', 'true');
    expect((await rows()).getAllByRole('button')).toHaveLength(4);
  });

  it('does not read every message in turn under the Unread filter', async () => {
    const { user } = setup();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Unread' }));
    await user.click((await rows()).getByRole('button', { name: /Brian Otieno/ }));
    expect(await folders().findByLabelText('3 unread')).toBeInTheDocument();
    // The opened message stays listed; nothing else was marked read.
    const list = await rows();
    expect(list.getAllByRole('button')).toHaveLength(4);
    expect(list.getByRole('button', { current: true })).toHaveTextContent('Brian Otieno');
  });

  it('searches the current folder', async () => {
    const { user } = setup();
    await rows();
    await user.type(screen.getByRole('searchbox', { name: 'Search mail' }), 'naivasha');
    const list = await rows();
    expect(list.getAllByRole('button')).toHaveLength(1);
    expect(list.getByRole('button')).toHaveTextContent('Photos from the weekend');
  });

  it('renders HTML mail only inside the sandboxed frame', async () => {
    const { user } = setup();
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const frame = await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(frame).toHaveAttribute('sandbox', '');
    expect(within(reader()).queryByText(/finished without errors/)).not.toBeInTheDocument();
  });

  it('flags a message from the toolbar', async () => {
    const { user } = setup();
    await rows();
    const flag = await within(reader()).findByRole('button', { name: 'Flag' });
    expect(flag).toHaveAttribute('aria-pressed', 'false');
    await user.click(flag);
    expect(await within(reader()).findByRole('button', { name: 'Flag', pressed: true })).toBeInTheDocument();
  });

  it('queues a quick reply and shows it in the Outbox count', async () => {
    const { user } = setup();
    await rows();
    const send = await within(reader()).findByRole('button', { name: 'Send' });
    expect(send).toBeDisabled();
    await user.type(within(reader()).getByLabelText('Quick reply to Wanjiru'), 'Signed, see attached.');
    await user.click(send);
    expect(await folders().findByLabelText('4 waiting')).toBeInTheDocument();
    expect(within(reader()).getByLabelText('Quick reply to Wanjiru')).toHaveValue('');
  });
});

describe('outbox', () => {
  async function openOutbox() {
    const ctx = setup();
    await rows();
    await ctx.user.click(folders().getByRole('button', { name: /^Outbox/ }));
    return { ...ctx, list: await rows('Outbox messages') };
  }

  it('selects the newest queued message first', async () => {
    const { list } = await openOutbox();
    expect(list.getByRole('button', { current: true })).toHaveTextContent('Client accounts');
  });

  it('shows the retrying banner with actions', async () => {
    const { user, list } = await openOutbox();
    expect(list.getByText('Retrying in 12 s')).toBeInTheDocument();
    await user.click(list.getByRole('button', { name: /Wanjiru Kamau/ }));
    const banner = await within(reader()).findByRole('status', { name: 'Delivery status' });
    expect(banner).toHaveTextContent('Waiting for a better connection');
    expect(within(banner).getByRole('button', { name: 'Send now' })).toBeInTheDocument();
    expect(within(banner).getByRole('button', { name: 'Cancel sending' })).toBeInTheDocument();
    expect(within(reader()).queryByRole('toolbar')).not.toBeInTheDocument();
  });

  it('shows progress while sending and no cancel', async () => {
    const { user, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Amina Hassan/ }));
    const banner = await within(reader()).findByRole('status', { name: 'Delivery status' });
    await within(banner).findByText('Sending, 64% uploaded');
    expect(within(banner).getByRole('progressbar', { name: 'Upload progress' })).toHaveAttribute('value', '64');
    expect(within(banner).queryByRole('button', { name: /Cancel/ })).not.toBeInTheDocument();
  });

  it('explains a too-large message', async () => {
    const { user, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Client accounts/ }));
    const banner = await within(reader()).findByRole('status', { name: 'Delivery status' });
    await within(banner).findByText(/attachments are too large/);
    expect(banner).toHaveTextContent('28 MB');
    expect(banner).toHaveTextContent('25 MB');
    expect(within(banner).queryByRole('button', { name: 'Send now' })).not.toBeInTheDocument();
  });

  it('removes a cancelled message', async () => {
    const { user, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Wanjiru Kamau/ }));
    const banner = await within(reader()).findByRole('status', { name: 'Delivery status' });
    await user.click(within(banner).getByRole('button', { name: 'Cancel sending' }));
    expect(await folders().findByLabelText('2 waiting')).toBeInTheDocument();
  });
});

describe('compose', () => {
  it('labels every field and validates recipients', async () => {
    const { user } = setup();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = screen.getByRole('form', { name: 'New message' });
    expect(within(form).getByLabelText('From')).toBeInTheDocument();
    expect(within(form).getByLabelText('To')).toHaveFocus();
    expect(within(form).getByLabelText('Subject')).toBeInTheDocument();
    expect(within(form).getByLabelText('Message')).toBeInTheDocument();

    await user.click(within(form).getByRole('button', { name: 'Send' }));
    expect(within(form).getByRole('alert')).toHaveTextContent('Add at least one recipient.');
    expect(within(form).getByLabelText('To')).toHaveAttribute('aria-invalid', 'true');

    await user.type(within(form).getByLabelText('To'), 'peter@client, ok@client.example');
    await user.click(within(form).getByRole('button', { name: 'Send' }));
    expect(within(form).getByRole('alert')).toHaveTextContent('Check peter@client.');
  });

  it('queues a valid message to the Outbox and closes', async () => {
    const { user } = setup();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = screen.getByRole('form', { name: 'New message' });
    await user.click(within(form).getByRole('button', { name: 'Add Cc or Bcc' }));
    expect(within(form).getByLabelText('Cc')).toBeInTheDocument();
    expect(within(form).getByLabelText('Bcc')).toBeInTheDocument();
    await user.type(within(form).getByLabelText('To'), 'peter@client.example');
    await user.type(within(form).getByLabelText('Subject'), 'Invoice INV-0144');
    await user.click(within(form).getByRole('button', { name: 'Send' }));
    expect(await folders().findByLabelText('4 waiting')).toBeInTheDocument();
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('opens a reply addressed to the sender', async () => {
    const { user } = setup();
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Reply' }));
    const form = screen.getByRole('form', { name: 'New message' });
    expect(within(form).getByLabelText('To')).toHaveValue('wanjiru@fitout.example');
    expect(within(form).getByLabelText('Subject')).toHaveValue(
      'Re: Revised quotation for the Westlands office fit-out',
    );
    expect(within(form).getByLabelText('Message')).toHaveFocus();
  });
});

describe('theme', () => {
  it('toggles and remembers the choice', async () => {
    const { user } = setup();
    await rows();
    expect(document.documentElement.dataset['theme']).toBe('light');
    await user.click(screen.getByRole('button', { name: 'Switch to dark mode' }));
    expect(document.documentElement.dataset['theme']).toBe('dark');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');
    expect(screen.getByRole('button', { name: 'Switch to light mode' })).toBeInTheDocument();
  });

  it('starts from a saved choice', async () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    setup();
    await rows();
    expect(document.documentElement.dataset['theme']).toBe('dark');
  });

  it('still toggles when storage is blocked', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    const { user } = setup();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Switch to dark mode' }));
    expect(document.documentElement.dataset['theme']).toBe('dark');
  });
});
