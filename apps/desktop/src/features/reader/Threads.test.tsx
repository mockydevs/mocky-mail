import { waitFor, within } from '@testing-library/react';

import type { MockMessage } from '../../api/mock/mockMailApi';
import { MOCK_MESSAGES } from '../../api/mock/mockData';
import { reader, renderApp, rows } from '../../test/renderApp';

const base = MOCK_MESSAGES.find((m) => m.id === 'm1');
if (!base) throw new Error('mock m1 missing');

/** A quote, my reply in Sent, and the client's answer, newest in the Inbox. */
const QUOTE: MockMessage = {
  ...base,
  id: 't1',
  subject: 'Kitchen quote',
  preview: 'Here is the quote for the kitchen.',
  text: 'Here is the quote for the kitchen.',
  html: null,
  attachments: [],
  hasAttachments: false,
  date: 1763000000,
  unread: false,
  messageIdHeader: '<kq-1@client.example>',
  references: [],
  inReplyTo: null,
};
const MINE: MockMessage = {
  ...QUOTE,
  id: 't2',
  folderId: 'work:sent',
  from: { name: null, address: 'you@work.example' },
  subject: 'Re: Kitchen quote',
  preview: 'Thanks, can we start Monday?',
  text: 'Thanks, can we start Monday?',
  date: 1763003600,
  messageIdHeader: '<kq-2@work.example>',
  references: ['<kq-1@client.example>'],
  inReplyTo: '<kq-1@client.example>',
};
const ANSWER: MockMessage = {
  ...QUOTE,
  id: 't3',
  subject: 'Re: Kitchen quote',
  preview: 'Monday is fine.',
  text: 'Monday is fine.',
  date: 1763999999,
  unread: true,
  messageIdHeader: '<kq-3@client.example>',
  references: ['<kq-1@client.example>', '<kq-2@work.example>'],
  inReplyTo: '<kq-2@work.example>',
};
const messages = [QUOTE, MINE, ANSWER, ...MOCK_MESSAGES];

describe('conversations', () => {
  it('shows a conversation once, with a count, newest first', async () => {
    renderApp({ messages });
    const list = await rows();
    const row = list.getByRole('button', { name: /Re: Kitchen quote/ });
    expect(within(row).getByLabelText('2 messages in this conversation')).toBeInTheDocument();
    expect(list.queryByRole('button', { name: /^(?!.*Re:).*Kitchen quote/ })).not.toBeInTheDocument();
  });

  it('shows earlier messages, sent ones included, and unfolds them', async () => {
    const { user } = renderApp({ messages });
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Re: Kitchen quote/ }));
    const history = within(await within(reader()).findByRole('region', { name: /^Earlier in this conversation, 2 messages/ }));
    const [first, second] = history.getAllByRole('button', { expanded: false });
    if (!first || !second) throw new Error('missing history rows');
    expect(second).toHaveTextContent('you@work.example');
    await user.click(first);
    expect(await history.findByText('Here is the quote for the kitchen.', { selector: '.thread__text' })).toBeInTheDocument();
  });

  it('archives the whole conversation in the folder', async () => {
    const { user, api } = renderApp({ messages });
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Re: Kitchen quote/ }));
    await user.click(within(reader()).getByRole('button', { name: 'Archive' }));
    await waitFor(async () => {
      const archive = await api.listMessages({ accountId: 'work', folderId: 'work:archive', role: null, filter: 'all', search: null, offset: 0, limit: 50 });
      expect(archive.items.map((m) => m.id).sort()).toEqual(['t1', 't3']);
    });
  });

  it('can be switched off in Settings', async () => {
    renderApp({ messages, settings: { theme: 'system', remoteImages: 'block', notifications: false, sound: false, undoSendSecs: 5, markRead: 'open', threads: false, focusedInbox: false, checkEveryMins: 2, markReadDelaySecs: 3, eventReminders: true, defaultReminderMins: 15, closeToTray: true, startAtLogin: false, device: { tray: true, startAtLogin: true } } });
    const list = await rows();
    expect(list.getAllByRole('button', { name: /Kitchen quote/ })).toHaveLength(2);
  });
});
