import { act, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import type { DraftSaved } from '../../api/types';
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

  it('keeps one saved copy: the draft id from each save is used by the next save and by send', async () => {
    const { user, api } = renderApp();
    await rows();
    const save = vi.spyOn(api, 'saveDraft');
    const send = vi.spyOn(api, 'sendMessage');
    expect(await unified().findByRole('button', { name: 'Drafts 1' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = composeForm();
    await user.type(form.getByLabelText('Subject'), 'Notes');
    await user.click(form.getByRole('button', { name: 'Save draft' }));
    expect(await form.findByText('Draft saved.')).toBeInTheDocument();
    expect(save.mock.calls[0]?.[0].draftMessageId).toBeNull();
    const first = await (save.mock.results[0]?.value as Promise<DraftSaved>);
    expect(await unified().findByRole('button', { name: 'Drafts 2' })).toBeInTheDocument();

    await user.type(form.getByLabelText('Subject'), ' for Monday');
    await user.click(form.getByRole('button', { name: 'Save draft' }));
    await vi.waitFor(() => {
      expect(save).toHaveBeenCalledTimes(2);
    });
    expect(save.mock.calls[1]?.[0].draftMessageId).toBe(first.draftMessageId);
    const second = await (save.mock.results[1]?.value as Promise<DraftSaved>);
    expect(second.draftMessageId).not.toBe(first.draftMessageId);
    expect(await unified().findByRole('button', { name: 'Drafts 2' })).toBeInTheDocument();

    await user.type(form.getByLabelText('To'), 'peter@client.example');
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(expect.objectContaining({ draftMessageId: second.draftMessageId }));
    });
    expect(await unified().findByRole('button', { name: 'Drafts 1' })).toBeInTheDocument();
  });

  it('closes without asking right after a save', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    await user.type(composeForm().getByLabelText('Subject'), 'Saved');
    await user.click(composeForm().getByRole('button', { name: 'Save draft' }));
    await composeForm().findByText('Draft saved.');
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('continues a saved draft with its recipients, threading and attachments, and sending replaces it', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(await unified().findByRole('button', { name: 'Drafts 1' }));
    await user.click(await within(reader()).findByRole('button', { name: 'Continue editing' }));
    const form = composeForm('Edit draft');
    expect(form.getByRole('button', { name: 'Remove Brian Otieno <brian@personal.example>' })).toBeInTheDocument();
    expect(form.getByText('Brian Otieno')).toBeInTheDocument();
    expect(form.getByLabelText('Subject')).toHaveValue('Re: Lunch on Friday?');
    expect(form.getByLabelText('Message')).toHaveValue('Friday works. 1pm at');
    expect(await form.findByText('Menu.pdf')).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({
          draftMessageId: '<d1@mock.example>',
          inReplyTo: '<lunch-1@personal.example>',
          references: ['<lunch-1@personal.example>'],
          attachments: [{ path: '/tmp/mocky-mail/d1/0-Menu.pdf', fileName: 'Menu.pdf', size: 220 * 1024 }],
        }),
      );
    });
    expect(await unified().findByRole('button', { name: 'Drafts' })).toBeInTheDocument();
  });

  it('forwards with the original attachments as removable chips', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(await within(reader()).findByRole('button', { name: 'Forward' }));
    const form = composeForm('Forward');
    expect(await form.findByText('Quotation-Westlands-rev2.pdf')).toBeInTheDocument();
    expect(form.getByText('Floor-plan.png')).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Remove Floor-plan.png' }));
    await user.type(form.getByLabelText('To'), 'amina@studio.example');
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({
          attachments: [
            { path: '/tmp/mocky-mail/m1/0-Quotation-Westlands-rev2.pdf', fileName: 'Quotation-Westlands-rev2.pdf', size: 412 * 1024 },
          ],
        }),
      );
    });
  });

  it('counts forwarded attachments in the size check', async () => {
    const { user } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Grace Njeri/ }));
    await user.click(await within(reader()).findByRole('button', { name: 'Forward' }));
    const form = composeForm('Forward');
    expect(await form.findByText('Naivasha.zip')).toBeInTheDocument();
    expect(form.getByText(/probably too large to send/)).toBeInTheDocument();
  });

  it('still opens a forward when the attachments cannot be copied', async () => {
    const { user, api } = renderApp();
    await rows();
    api.failNext('attachmentsForDraft', { kind: 'storage', message: 'No space left on the disk.' });
    await user.click(await within(reader()).findByRole('button', { name: 'Forward' }));
    const form = composeForm('Forward');
    const alert = await form.findByRole('alert');
    expect(alert).toHaveTextContent('Couldn\u2019t add the original attachments');
    expect(alert).toHaveTextContent('No space left on the disk.');
    expect(form.queryByRole('list', { name: 'Attached files' })).not.toBeInTheDocument();
    expect(form.getByRole('button', { name: 'Send' })).toBeEnabled();
  });

  it('shows that attachments are loading and blocks Send until they arrive', async () => {
    const { user, api } = renderApp();
    await rows();
    let finish: (files: { path: string; fileName: string; size: number }[]) => void = () => undefined;
    vi.spyOn(api, 'attachmentsForDraft').mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    await user.click(await within(reader()).findByRole('button', { name: 'Forward' }));
    const form = composeForm('Forward');
    expect(form.getByText('Adding the original attachments')).toBeInTheDocument();
    expect(form.getByRole('button', { name: 'Send' })).toBeDisabled();
    await act(async () => {
      finish([{ path: '/tmp/x.pdf', fileName: 'x.pdf', size: 10 }]);
      await Promise.resolve();
    });
    expect(await form.findByText('x.pdf')).toBeInTheDocument();
    expect(form.getByRole('button', { name: 'Send' })).toBeEnabled();
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
    const row = (label: string) => within(form.getByRole('list', { name: `${label} recipients` }));
    expect(row('To').getByRole('button', { name: 'Remove Wanjiru Kamau <wanjiru@fitout.example>' })).toBeInTheDocument();
    expect(row('Cc').getByRole('button', { name: 'Remove Amina Hassan <amina@studio.example>' })).toBeInTheDocument();
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
