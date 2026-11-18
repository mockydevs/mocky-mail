import { screen, waitFor, within } from '@testing-library/react';

import type { Contact } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

const PETER: Contact = {
  id: 'c-peter',
  accountId: 'work',
  uid: 'peter',
  name: 'Peter Mwangi',
  emails: ['peter@client.example', 'peter.m@gmail.example'],
  phones: ['+254 700 000 001'],
  organization: 'Westlands Fit-out',
  title: 'Site manager',
  notes: 'Calls before 10.',
  state: 'synced',
};

async function openContacts(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp({ contacts: [PETER], ...options });
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Contacts' }));
  const region = within(screen.getByRole('region', { name: 'Contacts' }));
  return { ...ctx, region };
}

describe('contacts', () => {
  it('lists, searches and shows a contact card', async () => {
    const { user, region } = await openContacts();
    const list = within(await region.findByRole('list', { name: 'Contacts' }));
    await user.click(list.getByRole('button', { name: /Peter Mwangi/ }));
    const card = within(region.getByRole('article', { name: 'Peter Mwangi' }));
    expect(card.getByText('Site manager, Westlands Fit-out')).toBeInTheDocument();
    expect(card.getByText('+254 700 000 001')).toBeInTheDocument();
    expect(card.getByText('Calls before 10.')).toBeInTheDocument();

    await user.type(region.getByRole('searchbox', { name: 'Search contacts' }), 'nobody');
    expect(await region.findByText('No contacts match.')).toBeInTheDocument();
  });

  it('adds a contact, synced to the account by default, and edits it', async () => {
    const { user, api, region } = await openContacts({ contacts: [] });
    await user.click(region.getByRole('button', { name: 'New contact' }));
    const form = within(region.getByRole('form', { name: 'New contact' }));
    await user.type(form.getByRole('textbox', { name: 'Name' }), 'Wanjiru Kamau');
    await user.type(form.getByRole('textbox', { name: 'Email 1' }), 'not-an-email');
    await user.click(form.getByRole('button', { name: 'Save contact' }));
    expect(await form.findByRole('alert')).toHaveTextContent("isn't an email address");

    await user.clear(form.getByRole('textbox', { name: 'Email 1' }));
    await user.type(form.getByRole('textbox', { name: 'Email 1' }), 'wanjiru@shop.example');
    expect(form.getByRole('combobox', { name: 'Save to' })).toHaveDisplayValue(/synced with the mail server/);
    await user.click(form.getByRole('button', { name: 'Save contact' }));

    const card = within(await region.findByRole('article', { name: 'Wanjiru Kamau' }));
    const saved = (await api.listContacts('wanjiru'))[0];
    expect(saved?.accountId).not.toBeNull();

    await user.click(card.getByRole('button', { name: 'Edit' }));
    const edit = within(region.getByRole('form', { name: 'Edit contact' }));
    await user.type(edit.getByRole('textbox', { name: 'Company' }), 'Kamau & Sons');
    await user.click(edit.getByRole('button', { name: 'Save contact' }));
    expect(await region.findByText('Kamau & Sons')).toBeInTheDocument();
  });

  it('deletes after asking', async () => {
    const { user, api, region } = await openContacts();
    await user.click(await region.findByRole('button', { name: /Peter Mwangi/ }));
    await user.click(region.getByRole('button', { name: 'Delete' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Delete Peter Mwangi?' }));
    expect(dialog.getByText(/also deletes the contact from .*address book on the server/)).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Delete contact' }));
    await waitFor(async () => {
      expect(await api.listContacts('')).toHaveLength(0);
    });
  });

  it('writes to a contact', async () => {
    const { user, region } = await openContacts();
    await user.click(await region.findByRole('button', { name: /Peter Mwangi/ }));
    await user.click(region.getByRole('button', { name: 'Write email' }));
    const form = within(await screen.findByRole('form', { name: 'New message' }));
    const to = within(form.getByRole('list', { name: 'To recipients' }));
    expect(to.getByText(/Peter Mwangi/)).toBeInTheDocument();
  });

  it('jumps to mail searched for the contact', async () => {
    const { user, region } = await openContacts();
    await user.click(await region.findByRole('button', { name: /Peter Mwangi/ }));
    await user.click(region.getByRole('button', { name: 'See emails' }));
    expect(await screen.findByRole('searchbox', { name: /Search/ })).toHaveValue('peter@client.example');
    expect(screen.queryByRole('region', { name: 'Contacts' })).not.toBeInTheDocument();
  });

  it('offers to save people from mail who are not contacts yet', async () => {
    const { user, region } = await openContacts({ contacts: [] });
    await user.type(region.getByRole('searchbox', { name: 'Search contacts' }), 'a');
    const people = within(await region.findByRole('list', { name: 'People from your mail' }));
    const [save] = people.getAllByRole('button', { name: /^Save .* as a contact$/ });
    if (!save) throw new Error('no suggestions');
    await user.click(save);
    expect(region.getByRole('form', { name: 'New contact' })).toBeInTheDocument();
    expect(region.getByRole('textbox', { name: 'Email 1' })).not.toHaveValue('');
  });

  it('says when an account’s contacts could not sync', async () => {
    const { region } = await openContacts({
      books: [{ accountId: 'work', state: 'error', message: 'The server didn’t answer.' }],
    });
    expect(await region.findByText(/contacts didn't sync\. The server didn’t answer\./)).toBeInTheDocument();
  });
});
