import { screen, waitFor, within } from '@testing-library/react';

import type { Contact } from '../../api/types';
import type * as PhotoModule from './photo';
import { renderApp, rows } from '../../test/renderApp';

vi.mock('./photo', async (importOriginal) => {
  const real = await importOriginal<typeof PhotoModule>();
  return { ...real, photoFromFile: vi.fn(() => Promise.resolve('data:image/jpeg;base64,AAAA')) };
});

const person = (id: string, name: string, email: string, extra: Partial<Contact> = {}): Contact => ({
  id,
  accountId: null,
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
  ...extra,
});

const PEOPLE = [
  person('c-amina', 'Amina Yusuf', 'amina@acme.example', { favorite: true }),
  person('c-peter', 'Peter Mwangi', 'peter@acme.example'),
  person('c-grace', 'Grace Njeri', 'grace@studio.example'),
];

async function openPeople(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp({ contacts: PEOPLE, ...options });
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Contacts' }));
  const region = within(screen.getByRole('region', { name: 'Contacts' }));
  await region.findByRole('list', { name: 'Contacts' });
  return { ...ctx, region };
}

const names = (list: HTMLElement) => within(list).getAllByRole('listitem').map((li) => li.querySelector('.contact-row__name')?.textContent);

describe('people', () => {
  it('lists favourites first, and stars and unstars people', async () => {
    const { user, api, region } = await openPeople();
    expect(names(region.getByRole('list', { name: 'Favourites' }))).toEqual(['Amina Yusuf']);
    expect(names(region.getByRole('list', { name: 'Contacts' }))).toEqual(['Grace Njeri', 'Peter Mwangi']);

    await user.click(region.getByRole('button', { name: /Peter Mwangi/ }));
    const card = within(region.getByRole('article', { name: 'Peter Mwangi' }));
    await user.click(card.getByRole('button', { name: 'Add to favourites' }));
    await waitFor(() => {
      expect(names(region.getByRole('list', { name: 'Favourites' }))).toEqual(['Amina Yusuf', 'Peter Mwangi']);
    });
    expect(card.getByRole('button', { name: 'Remove from favourites' })).toHaveAttribute('aria-pressed', 'true');
    expect((await api.getContact('c-peter')).favorite).toBe(true);
  });

  it('gives contacts categories and a photo', async () => {
    const { user, api, region } = await openPeople();
    await user.click(region.getByRole('button', { name: /Grace Njeri/ }));
    await user.click(within(region.getByRole('article', { name: 'Grace Njeri' })).getByRole('button', { name: 'Edit' }));
    const form = within(region.getByRole('form', { name: 'Edit contact' }));
    await user.click(form.getByRole('checkbox', { name: 'Clients' }));
    const file = new File(['png'], 'grace.png', { type: 'image/png' });
    await user.upload(form.getByLabelText('Choose a photo'), file);
    expect(await form.findByRole('button', { name: 'Change photo' })).toBeInTheDocument();
    await user.click(form.getByRole('button', { name: 'Save contact' }));

    const card = within(await region.findByRole('article', { name: 'Grace Njeri' }));
    expect(within(card.getByRole('list', { name: 'Categories' })).getByText('Clients')).toBeInTheDocument();
    const saved = await api.getContact('c-grace');
    expect(saved).toMatchObject({ categories: ['Clients'], photo: 'data:image/jpeg;base64,AAAA' });
    // The picture shows in the card and the list.
    expect(document.querySelectorAll(`img.avatar--photo[src="${saved.photo ?? ''}"]`).length).toBeGreaterThanOrEqual(2);

    await user.click(card.getByRole('button', { name: 'Edit' }));
    const again = within(region.getByRole('form', { name: 'Edit contact' }));
    await user.click(again.getByRole('button', { name: 'Remove photo' }));
    await user.click(again.getByRole('button', { name: 'Save contact' }));
    await waitFor(async () => {
      expect((await api.getContact('c-grace')).photo).toBeNull();
    });
  });

  it('makes contact groups, changes who is in them, and writes to everyone', async () => {
    const { user, api, region } = await openPeople();
    await user.click(region.getByRole('button', { name: 'New group' }));
    const naming = within(screen.getByRole('dialog', { name: 'New group' }));
    await user.type(naming.getByRole('textbox', { name: 'Group name' }), 'Acme team');
    await user.click(naming.getByRole('button', { name: 'Save' }));
    const group = within(await region.findByRole('article', { name: 'Acme team' }));
    expect(group.getByText('Nobody yet. Add people below.')).toBeInTheDocument();

    await user.selectOptions(group.getByRole('combobox', { name: 'Add someone' }), 'c-amina');
    await user.click(group.getByRole('button', { name: 'Add' }));
    await user.selectOptions(await group.findByRole('combobox', { name: 'Add someone' }), 'c-peter');
    await user.click(group.getByRole('button', { name: 'Add' }));
    const members = within(await group.findByRole('list', { name: 'People in Acme team' }));
    await waitFor(() => {
      expect(members.getAllByRole('listitem')).toHaveLength(2);
    });
    expect(names(region.getByRole('list', { name: 'Groups' }))).toEqual(['Acme team']);

    await user.click(members.getByRole('button', { name: 'Remove Peter Mwangi from Acme team' }));
    await waitFor(() => {
      expect(members.getAllByRole('listitem')).toHaveLength(1);
    });
    expect((await api.listContactGroups())[0]?.memberIds).toEqual(['c-amina']);

    await user.click(group.getByRole('button', { name: 'Rename' }));
    const rename = within(screen.getByRole('dialog', { name: 'Rename group' }));
    await user.clear(rename.getByRole('textbox', { name: 'Group name' }));
    await user.type(rename.getByRole('textbox', { name: 'Group name' }), 'Acme');
    await user.click(rename.getByRole('button', { name: 'Save' }));
    const renamed = within(await region.findByRole('article', { name: 'Acme' }));

    await user.click(renamed.getByRole('button', { name: 'Email group' }));
    const compose = await screen.findByRole('form', { name: 'New message' });
    const to = within(within(compose).getByRole('list', { name: 'To recipients' }));
    expect(to.getAllByRole('listitem').map((li) => li.getAttribute('title'))).toEqual(['Amina Yusuf <amina@acme.example>']);
  });

  it('deletes a group but keeps the people', async () => {
    const { user, api, region } = await openPeople({ groups: [{ id: 'g1', accountId: null, name: 'Suppliers', memberIds: ['c-peter'] }] });
    await user.click(within(region.getByRole('list', { name: 'Groups' })).getByRole('button', { name: /Suppliers/ }));
    await user.click(region.getByRole('button', { name: 'Delete group' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete group' }));
    await waitFor(() => {
      expect(region.queryByRole('list', { name: 'Groups' })).not.toBeInTheDocument();
    });
    expect(await api.listContactGroups()).toEqual([]);
    expect(await api.getContact('c-peter')).toBeDefined();
  });
});
