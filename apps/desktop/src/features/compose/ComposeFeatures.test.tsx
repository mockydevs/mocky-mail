import { render, screen, waitFor, within, type BoundFunctions, type queries } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { vi } from 'vitest';

import { MOCK_NOW, MOCK_SETTINGS } from '../../api/mock/mockData';
import { createMockMailApi, type MockOptions } from '../../api/mock/mockMailApi';
import type { Contact, Draft } from '../../api/types';
import { App } from '../../app/App';
import { messageText } from '../../test/editor';
import { renderApp, rows } from '../../test/renderApp';
import { composeWindowKey } from './ComposeWindow';
import { blankDraft } from './seed';
import { preloadEditor } from './editor/loader';

// Compose's editor is a chunk of its own: have it here before the first test.
beforeAll(async () => {
  await preloadEditor();
});

const contact = (id: string, name: string, email: string): Contact => ({
  id,
  accountId: 'work',
  uid: id,
  name,
  emails: [email],
  phones: [],
  organization: '',
  title: '',
  notes: '',
  favorite: false,
  categories: [],
  photo: null,
  state: 'synced',
});
const AMINA_W = contact('c-amina-w', 'Amina Wairimu', 'amina.w@site.example');
const PETER = contact('c-peter', 'Peter Mwangi', 'peter@client.example');

async function newMessage(options: MockOptions = {}) {
  const ctx = renderApp(options);
  await rows();
  const send = vi.spyOn(ctx.api, 'sendMessage');
  await ctx.user.click(screen.getByRole('button', { name: 'New message' }));
  const form = within(screen.getByRole('form', { name: 'New message' }));
  return { ...ctx, send, form };
}

const chips = (form: BoundFunctions<typeof queries>, field: string) =>
  within(form.getByRole('list', { name: `${field} recipients` }))
    .getAllByRole('button')
    .map((b) => b.getAttribute('aria-label'));

describe('Insert', () => {
  it('types a symbol picked from a searchable grid', async () => {
    const { user, form } = await newMessage();
    const body = await form.findByLabelText('Message');
    await user.click(body);
    await user.keyboard('Price 500');
    await user.click(form.getByRole('button', { name: 'Insert' }));
    await user.click(screen.getByRole('menuitem', { name: 'Symbol…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Insert a symbol' }));
    expect(dialog.getByRole('button', { name: 'Copyright ©' })).toBeInTheDocument();
    await user.type(dialog.getByRole('searchbox', { name: 'Search symbols' }), 'euro');
    expect(dialog.queryByRole('button', { name: 'Copyright ©' })).not.toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Euro €' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(messageText(body)).toBe('Price 500€');
  });

  it('attaches a contact as a card', async () => {
    const { user, form, api } = await newMessage({ contacts: [PETER, AMINA_W] });
    const card = vi.spyOn(api, 'contactCardAttachment');
    await user.click(form.getByRole('button', { name: 'Insert' }));
    await user.click(screen.getByRole('menuitem', { name: 'Contact card…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Attach a contact card' }));
    await user.type(dialog.getByRole('searchbox', { name: 'Search contacts' }), 'peter');
    await waitFor(() => {
      expect(dialog.queryByText('Amina Wairimu')).not.toBeInTheDocument();
    });
    await user.click(dialog.getByRole('button', { name: /Peter Mwangi/ }));
    expect(card).toHaveBeenCalledWith('c-peter');
    expect(await form.findByText('Peter Mwangi.vcf')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('attaches a message picked from recent mail', async () => {
    const { user, form, send } = await newMessage();
    await user.type(form.getByLabelText('To'), 'peter@client.example');
    await user.click(form.getByRole('button', { name: 'Insert' }));
    await user.click(screen.getByRole('menuitem', { name: 'Item…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Attach a message' }));
    expect(await dialog.findByRole('button', { name: /Revised quotation/ })).toBeInTheDocument();
    await user.type(dialog.getByRole('searchbox', { name: 'Search messages' }), 'lunch');
    await waitFor(() => {
      expect(dialog.queryByRole('button', { name: /Revised quotation/ })).not.toBeInTheDocument();
    });
    const [lunch] = dialog.getAllByRole('button', { name: /Lunch on Friday\?/ });
    if (!lunch) throw new Error('no Lunch on Friday? message');
    await user.click(lunch);
    expect(await form.findByText(/^(Re_ )?Lunch on Friday_\.eml$/)).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({ attachments: [expect.objectContaining({ fileName: expect.stringMatching(/\.eml$/) as unknown })] }),
      );
    });
  });
});

describe('Options', () => {
  it('asks for a read receipt', async () => {
    const { user, form, send } = await newMessage();
    await user.type(form.getByLabelText('To'), 'peter@client.example');
    await user.click(form.getByRole('button', { name: 'Options' }));
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Request a read receipt' }));
    expect(form.getByText(/A read receipt is requested/)).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(expect.objectContaining({ requestReadReceipt: true }));
    });
  });
});

describe('Check Names', () => {
  it('fills in a known name, asks which of several, and marks unknown ones', async () => {
    const { user, form, api } = await newMessage({ contacts: [AMINA_W] });
    const resolve = vi.spyOn(api, 'resolveRecipients');
    await user.type(form.getByLabelText('To'), 'Wanjiru, Amina, Nobody Here,');
    await user.click(form.getByRole('button', { name: 'Check names' }));
    expect(resolve).toHaveBeenCalledWith('Wanjiru, Amina, Nobody Here');

    const chooser = within(await screen.findByRole('dialog', { name: 'Who is “Amina”?' }));
    expect(chooser.getByRole('button', { name: /Amina Hassan/ })).toBeInTheDocument();
    await user.click(chooser.getByRole('button', { name: /Amina Wairimu/ }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(chips(form, 'To')).toEqual([
      'Remove Wanjiru Kamau <wanjiru@fitout.example>',
      'Remove Amina Wairimu <amina.w@site.example>',
      'Remove Nobody Here',
    ]);
    expect(form.getByRole('alert')).toHaveTextContent('Check Nobody Here. That does not look like an email address or the name of a contact.');
    expect(form.getByLabelText('To')).toHaveAttribute('aria-invalid', 'true');
  });

  it('runs with Alt+K', async () => {
    const { user, form, api } = await newMessage();
    const resolve = vi.spyOn(api, 'resolveRecipients');
    await user.type(form.getByLabelText('To'), 'Peter');
    await user.keyboard('{Alt>}k{/Alt}');
    await vi.waitFor(() => {
      expect(chips(form, 'To')).toEqual(['Remove Peter Mwangi <peter@client.example>']);
    });
    expect(resolve).toHaveBeenCalledWith('Peter');
    expect(form.getByRole('status')).toHaveTextContent('Names checked.');
  });

  it('runs on Send when a field has names, and sends once they are resolved', async () => {
    const { user, form, send } = await newMessage({ contacts: [AMINA_W] });
    await user.type(form.getByLabelText('To'), 'Wanjiru, Amina');
    await user.type(form.getByLabelText('Subject'), 'Site visit');
    await user.click(form.getByRole('button', { name: 'Send' }));
    const chooser = within(await screen.findByRole('dialog', { name: 'Who is “Amina”?' }));
    expect(send).not.toHaveBeenCalled();
    await user.click(chooser.getByRole('button', { name: /Amina Hassan/ }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({ to: 'Wanjiru Kamau <wanjiru@fitout.example>, Amina Hassan <amina@studio.example>' }),
      );
    });
  });

  it('blocks Send while a name is unknown or left unchosen', async () => {
    const { user, form, send, api } = await newMessage({ contacts: [AMINA_W] });
    const check = vi.spyOn(api, 'checkDraft');
    await user.type(form.getByLabelText('To'), 'Nobody Here');
    await user.click(form.getByRole('button', { name: 'Send' }));
    expect(await form.findByRole('alert')).toHaveTextContent('Check Nobody Here.');
    expect(check).not.toHaveBeenCalled();

    await user.click(form.getByRole('button', { name: 'Remove Nobody Here' }));
    await user.type(form.getByLabelText('To'), 'Amina');
    await user.click(form.getByRole('button', { name: 'Send' }));
    await user.click(within(await screen.findByRole('dialog', { name: 'Who is “Amina”?' })).getByRole('button', { name: 'Leave it for now' }));
    expect(form.getByRole('alert')).toHaveTextContent('Pick who Amina is, or type their address.');
    expect(send).not.toHaveBeenCalled();
    expect(check).not.toHaveBeenCalled();
  });
});

describe('Pop out', () => {
  it('carries on with the message in a window of its own, closing it here', async () => {
    const { user, form, api } = await newMessage();
    await user.type(form.getByLabelText('Subject'), 'Written elsewhere');
    await user.click(form.getByRole('button', { name: 'Pop out' }));
    await waitFor(() => {
      expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
    });
    // No "Discard this message?": the window holds it now.
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const [key] = api.openedComposeWindows;
    if (!key) throw new Error('no compose window');
    expect(await api.takeComposeDraft(key)).toEqual(expect.objectContaining({ subject: 'Written elsewhere' }));
  });

  it('is not offered on a phone, which has no windows', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    try {
      const { user } = renderApp({ phone: true });
      await rows();
      await user.click(screen.getByRole('button', { name: 'New message' }));
      const form = within(await screen.findByRole('form', { name: 'New message' }));
      expect(form.queryByRole('button', { name: 'Pop out' })).not.toBeInTheDocument();
      expect(form.getByRole('toolbar', { name: 'Formatting' }).closest('.rte')).toHaveClass('rte--phone');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('compose window (?compose=)', () => {
  const draft: Draft = { ...blankDraft('work'), to: 'Peter Mwangi <peter@client.example>', subject: 'From the window', bodyHtml: '<p>Hello <b>Peter</b></p>', bodyText: 'Hello Peter' };

  async function openWindow(settings = MOCK_SETTINGS) {
    const api = createMockMailApi({ settings });
    await api.openComposeWindow(draft);
    const key = api.openedComposeWindows[0] ?? '';
    const close = vi.spyOn(window, 'close').mockImplementation(() => undefined);
    const user = userEvent.setup();
    render(<App api={api} now={() => MOCK_NOW} composeKey={key} />);
    const form = within(await screen.findByRole('form', { name: 'New message' }));
    return { api, key, close, user, form };
  }

  it('reads the key from the address', () => {
    expect(composeWindowKey('?compose=compose-7')).toBe('compose-7');
    expect(composeWindowKey('?message=m1')).toBeNull();
  });

  it('shows only the message, filling the window, with the draft it was opened with', async () => {
    const { form } = await openWindow();
    expect(form.getByLabelText('Subject')).toHaveValue('From the window');
    expect(messageText(await form.findByLabelText('Message'))).toBe('Hello Peter');
    expect(screen.queryByRole('navigation', { name: 'Mailboxes' })).not.toBeInTheDocument();
    expect(screen.getByRole('form', { name: 'New message' })).toHaveClass('compose--window');
    expect(form.queryByRole('button', { name: 'Pop out' })).not.toBeInTheDocument();
  });

  it('sends, offers Undo while it can, and closes when it goes', async () => {
    const { api, user, form, close } = await openWindow();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(form.getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('You have 5 seconds to undo.')).toBeInTheDocument();
    expect(send).toHaveBeenCalledWith(expect.objectContaining({ subject: 'From the window', bodyHtml: '<p>Hello <strong>Peter</strong></p>' }));
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    const again = within(await screen.findByRole('form', { name: 'New message' }));
    expect(again.getByLabelText('Subject')).toHaveValue('From the window');
    expect(close).not.toHaveBeenCalled();
  });

  it('closes the window at once when there is nothing to undo', async () => {
    const { user, form, close } = await openWindow({ ...MOCK_SETTINGS, undoSendSecs: 0 });
    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(close).toHaveBeenCalled();
    });
  });

  it('asks before closing, since the message is held only there', async () => {
    const { user, form, close } = await openWindow();
    await user.click(form.getByRole('button', { name: 'Close' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Discard this message?' }));
    await user.click(dialog.getByRole('button', { name: 'Discard' }));
    expect(close).toHaveBeenCalled();
  });

  it('says so when the draft was already taken', async () => {
    const api = createMockMailApi();
    render(<App api={api} now={() => MOCK_NOW} composeKey="compose-404" />);
    expect(await screen.findByRole('alert')).toHaveTextContent('This message is not here any more');
  });
});
