import { createMockMailApi } from './mockMailApi';
import type { ListQuery } from '../types';

const inbox = (extra: Partial<ListQuery> = {}): ListQuery => ({
  accountId: 'work',
  folderId: 'work:inbox',
  role: null,
  filter: 'all',
  search: null,
  offset: 0,
  limit: 50,
  ...extra,
});

describe('mock backend: 0.5.0 mail handling', () => {
  it('sorts, puts pinned first in folders but not in search, and filters', async () => {
    const api = createMockMailApi();
    const ids = async (q: Partial<ListQuery>) => (await api.listMessages(inbox(q))).items.map((m) => m.id);
    expect(await ids({})).toEqual(['m1', 'm2', 'm5', 'm6']);
    expect(await ids({ descending: false })).toEqual(['m6', 'm5', 'm2', 'm1']);
    expect(await ids({ sort: 'from', descending: false })).toEqual(['m2', 'm5', 'm6', 'm1']);
    expect(await ids({ sort: 'size' })).toEqual(['m1', 'm6', 'm2', 'm5']);
    await api.setFlags(['m6'], { pinned: true });
    expect(await ids({})).toEqual(['m6', 'm1', 'm2', 'm5']);
    // A search lists by the sort alone.
    expect(await ids({ search: 'work.example' })).toEqual(['m1', 'm2', 'm5', 'm6']);
    expect(await ids({ filter: 'pinned' })).toEqual(['m6']);
    expect(await ids({ filter: 'attachments' })).toEqual(['m6', 'm1']);
  });

  it('sets and clears a flag due date', async () => {
    const api = createMockMailApi();
    await api.setFlags(['m2'], { flagDue: 1_900_000_000 });
    expect(await api.getMessage('m2', false)).toMatchObject({ flagged: true, flagDue: 1_900_000_000 });
    await api.setFlags(['m2'], { flagDue: null });
    expect(await api.getMessage('m2', false)).toMatchObject({ flagged: true, flagDue: null });
    await api.setFlags(['m4'], { flagged: false });
    expect(await api.getMessage('m4', false)).toMatchObject({ flagged: false, flagDue: null });
  });

  it('copies within an account only, and moves junk to Spam and back to the Inbox', async () => {
    const api = createMockMailApi();
    await api.copyMessages(['m2'], { folderId: 'work:Clients' });
    expect((await api.listMessages(inbox({ folderId: 'work:Clients' }))).total).toBe(2);
    expect((await api.listMessages(inbox())).total).toBe(4);
    await expect(api.copyMessages(['m2'], { folderId: 'personal:Travel' })).rejects.toMatchObject({ kind: 'invalid' });
    await api.markJunk(['m2'], true);
    expect((await api.getMessage('m2', false)).folderId).toBe('work:junk');
    await api.markJunk(['m2'], false);
    expect((await api.getMessage('m2', false)).folderId).toBe('work:inbox');
  });

  it('keeps category names unique, renames them on messages and removes deleted ones', async () => {
    const api = createMockMailApi();
    await expect(api.saveCategory({ name: 'CLIENTS', color: '#000000' }, null)).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.saveCategory({ name: 'X', color: 'red' }, null)).rejects.toMatchObject({ kind: 'invalid' });
    await api.saveCategory({ name: 'Customers', color: '#123456' }, 'Clients');
    expect((await api.getMessage('m1', false)).categories).toEqual(['Customers']);
    await expect(api.setCategories(['m2'], ['Nope'], [])).rejects.toMatchObject({ kind: 'not_found' });
    await api.setCategories(['m2'], ['Family', 'customers'], []);
    expect((await api.getMessage('m2', false)).categories).toEqual(['Family', 'customers']);
    await api.deleteCategory('Family');
    expect((await api.getMessage('m2', false)).categories).toEqual(['customers']);
  });

  it('orders accounts, keeps a sync window and favourites', async () => {
    const api = createMockMailApi({ favorites: ['work:archive'] });
    await expect(api.reorderAccounts(['personal'])).rejects.toMatchObject({ kind: 'invalid' });
    await api.reorderAccounts(['personal', 'work']);
    expect((await api.listAccounts()).map((a) => [a.id, a.sortOrder])).toEqual([
      ['personal', 0],
      ['work', 1],
    ]);
    await expect(api.updateAccount('work', { syncMonths: 7 })).rejects.toMatchObject({ kind: 'invalid' });
    expect((await api.updateAccount('work', { syncMonths: 3 })).syncMonths).toBe(3);
    expect((await api.updateAccount('work', { syncMonths: null })).syncMonths).toBeNull();
    expect((await api.listFolders('work')).filter((f) => f.favorite).map((f) => f.id)).toEqual(['work:archive']);
    await api.setFolderFavorite('work:Clients', true);
    expect((await api.listFolders('work')).filter((f) => f.favorite).map((f) => f.id)).toEqual(['work:archive', 'work:Clients']);
  });

  it('opens message windows on computers only', async () => {
    const api = createMockMailApi();
    await api.openMessageWindow('m1');
    expect(api.openedWindows).toEqual(['m1']);
    await expect(createMockMailApi({ phone: true }).openMessageWindow('m1')).rejects.toMatchObject({ kind: 'invalid' });
  });
});

describe('mock backend: 0.7.0 compose', () => {
  it('checks names against contacts and people in the mail', async () => {
    const api = createMockMailApi({
      contacts: [
        { id: 'c1', accountId: 'work', uid: 'c1', name: 'Amina Wairimu', emails: ['amina.w@site.example'], phones: [], organization: '', title: '', notes: '', favorite: false, categories: [], photo: null, state: 'synced' },
      ],
    });
    const check = await api.resolveRecipients('Wanjiru, peter@client.example, "Boss" <boss@work.example>, Amina, nobody here');
    expect(check.resolved).toEqual([
      { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
      { name: 'Peter Mwangi', address: 'peter@client.example' },
      { name: 'Boss', address: 'boss@work.example' },
    ]);
    expect(check.ambiguous).toEqual([
      {
        input: 'Amina',
        candidates: expect.arrayContaining([
          { name: 'Amina Wairimu', address: 'amina.w@site.example' },
          { name: 'Amina Hassan', address: 'amina@studio.example' },
        ]) as unknown,
      },
    ]);
    expect(check.invalid).toEqual(['nobody here']);
    // An exact name of several wins, as in the backend.
    expect((await api.resolveRecipients('Amina Hassan')).resolved).toEqual([{ name: 'Amina Hassan', address: 'amina@studio.example' }]);
  });

  it('makes contact cards and messages into attachments', async () => {
    const api = createMockMailApi({
      contacts: [{ id: 'c1', accountId: null, uid: 'c1', name: 'Peter Mwangi', emails: ['peter@client.example'], phones: [], organization: '', title: '', notes: '', favorite: false, categories: [], photo: null, state: 'synced' }],
    });
    expect(await api.contactCardAttachment('c1')).toEqual(expect.objectContaining({ fileName: 'Peter Mwangi.vcf' }));
    await expect(api.contactCardAttachment('gone')).rejects.toMatchObject({ kind: 'not_found' });
    expect(await api.messageAsAttachment('m3')).toEqual(expect.objectContaining({ fileName: 'Lunch on Friday_.eml' }));
  });

  it('answers a read receipt once', async () => {
    const api = createMockMailApi();
    expect((await api.getMessage('m6', false)).readReceipt).toEqual({ requestedBy: 'peter@client.example', answered: false });
    await api.answerReadReceipt('m6', true);
    expect((await api.getMessage('m6', false)).readReceipt?.answered).toBe(true);
    await expect(api.answerReadReceipt('m6', false)).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.answerReadReceipt('m1', true)).rejects.toMatchObject({ kind: 'invalid' });
    expect(api.answeredReceipts).toEqual([{ messageId: 'm6', send: true }]);
  });

  it('hands each compose window its draft once; phones have none', async () => {
    const api = createMockMailApi();
    const draft = { accountId: 'work', to: '', cc: '', bcc: '', subject: 'A', bodyText: '', bodyHtml: '<p></p>', inReplyTo: null, references: [], attachments: [], replyToMessageId: null, draftMessageId: null };
    await api.openComposeWindow(draft);
    await api.openComposeWindow({ ...draft, subject: 'B' });
    const [a, b] = api.openedComposeWindows;
    expect(a).not.toBe(b);
    expect((await api.takeComposeDraft(b ?? ''))?.subject).toBe('B');
    expect(await api.takeComposeDraft(b ?? '')).toBeNull();
    expect((await api.takeComposeDraft(a ?? ''))?.subject).toBe('A');
    await expect(createMockMailApi({ phone: true }).openComposeWindow(draft)).rejects.toMatchObject({ kind: 'invalid' });
  });
});
