import { screen, waitFor, within } from '@testing-library/react';

import type { MockMessage } from '../../api/mock/mockMailApi';
import { MOCK_MESSAGES, MOCK_SETTINGS } from '../../api/mock/mockData';
import { reader, renderApp, rows } from '../../test/renderApp';

const base = MOCK_MESSAGES.find((m) => m.id === 'm1');
if (!base) throw new Error('mock m1 missing');
const NEWS: MockMessage = {
  ...base,
  id: 'n1',
  from: { name: 'Bank alerts', address: 'noreply@bank.example' },
  subject: 'Your statement is ready',
  preview: 'Download your statement.',
  text: 'Download your statement.',
  html: null,
  attachments: [],
  hasAttachments: false,
  date: 1999999999,
  messageIdHeader: '<n1@bank.example>',
  references: [],
  inReplyTo: null,
};
const messages = [NEWS, ...MOCK_MESSAGES];
const settings = { ...MOCK_SETTINGS, focusedInbox: true };

describe('Focused Inbox', () => {
  it('is off unless turned on', async () => {
    renderApp({ messages });
    const list = await rows();
    expect(list.getByRole('button', { name: /Your statement is ready/ })).toBeInTheDocument();
    expect(screen.queryByRole('tablist', { name: 'Focused Inbox' })).not.toBeInTheDocument();
  });

  it('puts automated mail in Other', async () => {
    const { user } = renderApp({ messages, settings });
    const list = await rows();
    expect(list.queryByRole('button', { name: /Your statement is ready/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('tab', { name: 'Other' }));
    expect(await (await rows()).findByRole('button', { name: /Your statement is ready/ })).toBeInTheDocument();
  });

  it('moves a sender for good from the message menu', async () => {
    const { user } = renderApp({ messages, settings });
    await rows();
    await user.click(screen.getByRole('tab', { name: 'Other' }));
    await user.click(await (await rows()).findByRole('button', { name: /Your statement is ready/ }));
    await user.click(await within(reader()).findByRole('button', { name: 'More actions' }));
    await user.click(within(reader()).getByRole('button', { name: 'Always move this sender to Focused' }));
    expect(await screen.findByText('Mail from noreply@bank.example will go there from now on.')).toBeInTheDocument();
    await user.click(screen.getByRole('tab', { name: 'Focused' }));
    await waitFor(async () => {
      expect((await rows()).getByRole('button', { name: /Your statement is ready/ })).toBeInTheDocument();
    });
  });
});
