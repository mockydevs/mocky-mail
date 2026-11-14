import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { reader, renderApp, rows, unified } from '../../test/renderApp';

const MB = 1024 * 1024;
const composeForm = (name = 'New message') => within(screen.getByRole('form', { name }));

describe('compose', () => {
  it('labels every field and validates recipients before calling the backend', async () => {
    const { user, api } = renderApp();
    await rows();
    const check = vi.spyOn(api, 'checkDraft');
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = composeForm();
    expect(form.getByLabelText('From')).toBeInTheDocument();
    expect(form.getByLabelText('To')).toHaveFocus();
    expect(form.getByLabelText('Subject')).toBeInTheDocument();
    expect(form.getByLabelText('Message')).toBeInTheDocument();

    await user.click(form.getByRole('button', { name: 'Send' }));
    expect(form.getByRole('alert')).toHaveTextContent('Add at least one recipient.');
    await user.type(form.getByLabelText('To'), 'peter@client, ok@client.example');
    await user.click(form.getByRole('button', { name: 'Send' }));
    expect(form.getByRole('alert')).toHaveTextContent('Check peter@client.');
    expect(check).not.toHaveBeenCalled();
  });

  it('attaches files and sends to the Outbox', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = composeForm();
    await user.type(form.getByLabelText('To'), 'Peter Mwangi <peter@client.example>');
    await user.type(form.getByLabelText('Subject'), 'Invoice INV-0144');
    await user.click(form.getByRole('button', { name: 'Attach files' }));
    expect(await form.findByText('INV-0144.pdf')).toBeInTheDocument();
    expect(form.getByText(/accepts messages up to 25 MB/)).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Send' }));
    expect(await unified().findByRole('button', { name: 'Outbox 4 waiting' })).toBeInTheDocument();
    expect(send).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: 'work',
        to: 'Peter Mwangi <peter@client.example>',
        attachments: [{ path: '/home/you/Documents/INV-0144.pdf', fileName: 'INV-0144.pdf', size: 186 * 1024 }],
      }),
    );
    expect(screen.getByText('Message queued')).toBeInTheDocument();
  });

  it('rejects an attachment the account cannot send and does not queue it', async () => {
    const { user, api } = renderApp({ picked: [{ path: '/v/scan.pdf', fileName: 'scan.pdf', size: 30 * MB }] });
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = composeForm();
    await user.type(form.getByLabelText('To'), 'accounts@client.example');
    await user.click(form.getByRole('button', { name: 'Attach files' }));
    expect(await form.findByText(/probably too large to send/)).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Send' }));
    const alert = await form.findByRole('alert');
    expect(alert).toHaveTextContent('Too large to send');
    expect(alert).toHaveTextContent('the server accepts 26214400');
    expect(send).not.toHaveBeenCalled();
    await user.click(form.getByRole('button', { name: 'Remove scan.pdf' }));
    expect(form.queryByText('scan.pdf')).not.toBeInTheDocument();
  });

  it('saves a draft', async () => {
    const { user } = renderApp();
    await rows();
    expect(await unified().findByRole('button', { name: 'Drafts 1' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = composeForm();
    await user.type(form.getByLabelText('Subject'), 'Notes');
    await user.click(form.getByRole('button', { name: 'Save draft' }));
    expect(await form.findByText('Draft saved.')).toBeInTheDocument();
    expect(await unified().findByRole('button', { name: 'Drafts 2' })).toBeInTheDocument();
  });

  it('asks before discarding changes on Esc', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    await user.type(composeForm().getByLabelText('Subject'), 'Half written');
    await user.keyboard('{Escape}');
    const dialog = screen.getByRole('dialog', { name: 'Discard this message?' });
    await user.click(within(dialog).getByRole('button', { name: 'Keep editing' }));
    expect(composeForm().getByLabelText('Subject')).toHaveValue('Half written');
    await user.keyboard('{Escape}');
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Discard' }));
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('replies to all with threading headers and a quote, leaving me out', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(await within(reader()).findByRole('button', { name: 'Reply all' }));
    const form = composeForm('Reply all');
    expect(form.getByLabelText('To')).toHaveValue('Wanjiru Kamau <wanjiru@fitout.example>');
    expect(form.getByLabelText('Cc')).toHaveValue('Amina Hassan <amina@studio.example>');
    expect(form.getByLabelText('Subject')).toHaveValue('Re: Revised quotation for the Westlands office fit-out');
    expect(form.getByLabelText('Message')).toHaveFocus();
    expect(form.getByLabelText<HTMLTextAreaElement>('Message').value).toContain('> Thanks,\n> Wanjiru');
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({
          inReplyTo: '<m1@mock.example>',
          references: ['<q-2040@fitout.example>', '<m1@mock.example>'],
          replyToMessageId: 'm1',
        }),
      );
    });
  });

  it('forwards without recipients', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Forward' }));
    const form = composeForm('Forward');
    expect(form.getByLabelText('To')).toHaveValue('');
    expect(form.getByLabelText('Subject')).toHaveValue('Fwd: Revised quotation for the Westlands office fit-out');
    expect(form.getByLabelText<HTMLTextAreaElement>('Message').value).toContain('---------- Forwarded message ----------');
  });
});
