import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_MESSAGES, MOCK_SETTINGS } from '../../api/mock/mockData';
import type { Settings } from '../../api/types';
import { reader, renderApp, rows } from '../../test/renderApp';

/** Open Peter's "Invoice INV-0142 paid", which asks for a read receipt. */
async function openAsking(readReceipts: Settings['readReceipts'] = 'ask', messages = MOCK_MESSAGES) {
  const ctx = renderApp({ settings: { ...MOCK_SETTINGS, readReceipts }, messages });
  await ctx.user.click((await rows()).getByRole('button', { name: /Invoice INV-0142 paid/ }));
  await within(reader()).findByRole('heading', { name: 'Invoice INV-0142 paid' });
  return ctx;
}

describe('read receipts asked for by a sender', () => {
  it('asks, and sends one when told to', async () => {
    const { user, api } = await openAsking();
    const bar = within(await within(reader()).findByRole('group', { name: 'Read receipt' }));
    expect(bar.getByText('peter@client.example asked to know when you read this.')).toBeInTheDocument();
    await user.click(bar.getByRole('button', { name: 'Send receipt' }));
    expect(await within(reader()).findByText('Read receipt sent.')).toBeInTheDocument();
    expect(api.answeredReceipts).toEqual([{ messageId: 'm6', send: true }]);
    expect(within(reader()).queryByRole('group', { name: 'Read receipt' })).not.toBeInTheDocument();
  });

  it("can decline, and doesn't ask again for that message", async () => {
    const { user, api } = await openAsking();
    await user.click(await within(reader()).findByRole('button', { name: 'Don’t send' }));
    expect(await within(reader()).findByText('No read receipt was sent.')).toBeInTheDocument();
    expect(api.answeredReceipts).toEqual([{ messageId: 'm6', send: false }]);
    expect((await api.getMessage('m6', false)).readReceipt).toEqual({ requestedBy: 'peter@client.example', answered: true });
  });

  it('does not ask about one already answered', async () => {
    const messages = MOCK_MESSAGES.map((m) =>
      m.id === 'm6' ? { ...m, readReceipt: { requestedBy: 'peter@client.example', answered: true } } : m,
    );
    await openAsking('ask', messages);
    expect(within(reader()).queryByRole('group', { name: 'Read receipt' })).not.toBeInTheDocument();
  });

  it('leaves it to the backend with "Always send" and does not ask', async () => {
    const { api } = await openAsking('always');
    await vi.waitFor(() => {
      expect(api.answeredReceipts).toEqual([{ messageId: 'm6', send: true }]);
    });
    expect(within(reader()).queryByRole('group', { name: 'Read receipt' })).not.toBeInTheDocument();
  });

  it('never tells the sender with "Never send"', async () => {
    const { api } = await openAsking('never');
    const answer = vi.spyOn(api, 'answerReadReceipt');
    expect(within(reader()).queryByRole('group', { name: 'Read receipt' })).not.toBeInTheDocument();
    expect(answer).not.toHaveBeenCalled();
    expect(api.answeredReceipts).toEqual([]);
  });

  it('is chosen in Settings › Reading', async () => {
    const { user, api } = renderApp();
    await rows();
    const save = vi.spyOn(api, 'setSettings');
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const settings = within(screen.getByRole('region', { name: 'Settings' }));
    const choice = settings.getByRole('combobox', { name: 'Read receipts' });
    expect(choice).toHaveDisplayValue('Ask each time');
    expect(within(choice).getAllByRole('option').map((o) => o.textContent)).toEqual(['Ask each time', 'Always send', 'Never send']);
    await user.selectOptions(choice, 'Never send');
    await vi.waitFor(() => {
      expect(save).toHaveBeenCalledWith(expect.objectContaining({ readReceipts: 'never' }));
    });
  });
});
