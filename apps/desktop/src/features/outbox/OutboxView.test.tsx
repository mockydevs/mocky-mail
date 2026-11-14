import { act, screen, within } from '@testing-library/react';

import { reader, renderApp, rows, unified } from '../../test/renderApp';

async function openOutbox() {
  const ctx = renderApp();
  await rows();
  await ctx.user.click(await unified().findByRole('button', { name: 'Outbox 3 waiting' }));
  return { ...ctx, list: await rows('Outbox') };
}

const banner = () => within(within(reader()).getByRole('status', { name: 'Delivery status' }));

describe('outbox', () => {
  it('lists waiting messages newest first with their state', async () => {
    const { list } = await openOutbox();
    const items = list.getAllByRole('button');
    expect(items.map((b) => b.textContent)).toEqual([
      expect.stringContaining('Too large to send'),
      expect.stringContaining('Sending 64%'),
      expect.stringContaining('Retrying in 12 s'),
    ]);
  });

  it('explains a too-large message and offers edit instead of retry', async () => {
    const { user } = await openOutbox();
    expect(banner().getByText(/the message is too large/)).toBeInTheDocument();
    expect(banner().queryByRole('button', { name: 'Send now' })).not.toBeInTheDocument();
    await user.click(banner().getByRole('button', { name: 'Edit message' }));
    const form = within(screen.getByRole('form', { name: 'Edit message' }));
    expect(form.getByLabelText('Subject')).toHaveValue('Invoice INV-0143');
    expect(await unified().findByRole('button', { name: 'Outbox 2 waiting' })).toBeInTheDocument();
  });

  it('shows progress while sending and allows no changes', async () => {
    const { user, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Amina Hassan/ }));
    expect(banner().getByRole('progressbar', { name: 'Upload progress' })).toHaveAttribute('value', '64');
    expect(banner().queryByRole('button')).not.toBeInTheDocument();
  });

  it('sends a retrying message now, or cancels it', async () => {
    const { user, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Wanjiru Kamau/ }));
    expect(banner().getByText('Waiting for a better connection')).toBeInTheDocument();
    await user.click(banner().getByRole('button', { name: 'Send now' }));
    expect(await (await rows('Outbox')).findAllByText('Sending 0%')).toHaveLength(1);

    await user.click((await rows('Outbox')).getByRole('button', { name: /Client accounts/ }));
    await user.click(banner().getByRole('button', { name: 'Delete message' }));
    expect(await unified().findByRole('button', { name: 'Outbox 2 waiting' })).toBeInTheDocument();
  });

  it('updates live from outbox events', async () => {
    const { api } = await openOutbox();
    act(() => {
      api.emit('mail://outbox-changed', { entries: [] });
    });
    expect(await screen.findByText('Outbox is empty')).toBeInTheDocument();
    expect(unified().getByRole('button', { name: 'Outbox' })).toBeInTheDocument();
  });

  it('shows backend errors inline', async () => {
    const { user, api, list } = await openOutbox();
    await user.click(list.getByRole('button', { name: /Wanjiru Kamau/ }));
    api.failNext('outboxSendNow', { kind: 'storage', message: 'Could not write the queue.' });
    await user.click(banner().getByRole('button', { name: 'Send now' }));
    expect(await within(reader()).findByRole('alert')).toHaveTextContent('Could not write the queue.');
  });
});
