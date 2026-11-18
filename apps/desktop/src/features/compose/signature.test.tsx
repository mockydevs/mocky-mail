import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { vi } from 'vitest';

import { reader, renderApp, rows } from '../../test/renderApp';
import { signatureBlock, swapSignature, withSignature } from './seed';

const WORK = { html: '<b>Don Onyango</b><br>Mocky Digital', text: 'Don Onyango\nMocky Digital' };
const PERSONAL = { html: 'Don', text: 'Don' };

describe('signature placement', () => {
  it('goes under where you type, and above the quoted original', () => {
    expect(withSignature('', WORK.text)).toBe(`\n\n${signatureBlock(WORK.text)}`);
    const reply = '\n\nOn Tue, Amina wrote:\n> Lunch?';
    const body = withSignature(reply, WORK.text);
    expect(body.indexOf(signatureBlock(WORK.text))).toBeLessThan(body.indexOf('Amina wrote'));
    // Never twice.
    expect(withSignature(body, WORK.text)).toBe(body);
    expect(withSignature(reply, null)).toBe(reply);
  });

  it('swaps with the account, and leaves a signature you edited alone', () => {
    const body = withSignature('', WORK.text);
    expect(swapSignature(body, WORK.text, PERSONAL.text)).toBe(withSignature('', PERSONAL.text));
    expect(swapSignature(body, WORK.text, null)).toBe('');
    const edited = body.replace('Mocky Digital', 'Mocky');
    expect(swapSignature(edited, WORK.text, PERSONAL.text)).toBe(edited);
  });
});

describe('signature in the app', () => {
  it('is added to a new message without counting as an unsaved change, and follows From', async () => {
    const { user, api } = renderApp({ signatures: { work: WORK, personal: PERSONAL } });
    await rows();
    const accounts = await api.listAccounts();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = within(screen.getByRole('form', { name: 'New message' }));
    const body = form.getByLabelText<HTMLTextAreaElement>('Message');
    const first = accounts[0];
    const other = accounts[1];
    if (!first || !other) throw new Error('mock accounts');
    const expected = (id: string) => (id === 'work' ? WORK.text : id === 'personal' ? PERSONAL.text : null);
    await waitFor(() => {
      expect(body.value).toBe(withSignature('', expected(first.id)));
    });
    await user.selectOptions(form.getByLabelText('From'), other.id);
    await waitFor(() => {
      expect(body.value).toBe(withSignature('', expected(other.id)));
    });
  });

  it('is added to quick replies', async () => {
    const { user, api } = renderApp({ signatures: { work: WORK, personal: WORK } });
    const send = vi.spyOn(api, 'sendMessage');
    await rows();
    const box = await within(reader()).findByLabelText(/Quick reply to/);
    await user.type(box, 'Approved.');
    await user.click(within(reader()).getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(send).toHaveBeenCalled();
    });
    const body = send.mock.calls[0]?.[0].bodyText ?? '';
    expect(body.startsWith('Approved.')).toBe(true);
    expect(body).toContain(signatureBlock(WORK.text));
  });

  it('is pasted from webmail in Settings, cleaned by the backend, and saved', async () => {
    const { user, api } = renderApp();
    await rows();
    const save = vi.spyOn(api, 'setSignature');
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const editors = await screen.findAllByRole('textbox', { name: 'Signature' });
    const editor = editors[0];
    if (!editor) throw new Error('no signature editor');
    await waitFor(() => {
      expect(editor).toHaveAttribute('contenteditable', 'true');
    });
    fireEvent.paste(editor, {
      clipboardData: {
        getData: (type: string) => (type === 'text/html' ? '<b>Don</b><script>steal()</script>' : 'Don'),
      },
    });
    await screen.findByText('Signature pasted and saved.');
    expect(save).toHaveBeenCalledWith(expect.any(String), '<b>Don</b><script>steal()</script>');
    // What is shown is what the backend returned, never the raw paste.
    expect(editor.innerHTML).toBe('<b>Don</b>');
    const [remove] = screen.getAllByRole('button', { name: 'Remove signature' });
    if (!remove) throw new Error('no remove button');
    await user.click(remove);
    await screen.findByText('Signature removed.');
    expect(editor.innerHTML).toBe('');
  });
});

describe('signature picture', () => {
  const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

  async function signatureSettings(options: Parameters<typeof renderApp>[0] = {}) {
    const ctx = renderApp(options);
    await rows();
    await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
    const region = within(screen.getByRole('region', { name: 'Settings' }));
    return { ...ctx, region };
  }

  it('can be added, resized and removed in Settings', async () => {
    const { user, region } = await signatureSettings();
    const [add] = await region.findAllByRole('button', { name: 'Add a picture' });
    if (!add) throw new Error('no Add a picture button');
    await user.click(add);
    const img = await region.findByRole('img', { name: 'Your signature' });
    expect(img.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
    expect(await region.findByText(/Picture added/)).toBeInTheDocument();

    const size = region.getByRole('combobox', { name: 'Size in messages' });
    await user.selectOptions(size, '160');
    await waitFor(() => {
      expect(region.getByRole('img', { name: 'Your signature' })).toHaveStyle({ width: '160px' });
    });

    await user.click(region.getByRole('button', { name: 'Remove picture' }));
    await waitFor(() => {
      expect(region.queryByRole('img', { name: 'Your signature' })).not.toBeInTheDocument();
    });
  });

  it('keeps the picture when the words are changed, and shows a clear error for other files', async () => {
    const { user, api, region } = await signatureSettings({
      signatures: {
        work: {
          ...WORK,
          image: { mimeType: 'image/png', data: PNG, width: 240, naturalWidth: 480, naturalHeight: 120 },
        },
      },
    });
    expect(await region.findByRole('img', { name: 'Your signature' })).toBeInTheDocument();
    api.failNext('setSignatureImage', {
      kind: 'invalid',
      message: 'Use a PNG, JPG or GIF picture. From Illustrator, use File > Export > Export As > PNG.',
    });
    await user.click(region.getByRole('button', { name: 'Replace picture' }));
    expect(await region.findByRole('alert')).toHaveTextContent('Export As > PNG');
    const saved = await api.setSignature('work', '<b>Don</b>');
    expect(saved?.image?.width).toBe(240);
  });

  it('is previewed under a new message while its block is there', async () => {
    const { user } = renderApp({
      signatures: {
        work: { ...WORK, image: { mimeType: 'image/png', data: PNG, width: 320, naturalWidth: 640, naturalHeight: 160 } },
        personal: { ...WORK, image: { mimeType: 'image/png', data: PNG, width: 320, naturalWidth: 640, naturalHeight: 160 } },
      },
    });
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = within(screen.getByRole('form', { name: 'New message' }));
    const preview = await form.findByRole('img', { name: 'Your signature' });
    expect(preview).toHaveStyle({ width: '240px' });
    // Deleting the signature from the body drops the picture too.
    const body = form.getByLabelText<HTMLTextAreaElement>('Message');
    fireEvent.change(body, { target: { value: 'Just words' } });
    await waitFor(() => {
      expect(form.queryByRole('img', { name: 'Your signature' })).not.toBeInTheDocument();
    });
  });
});
