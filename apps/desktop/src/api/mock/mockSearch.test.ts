import { vi } from 'vitest';

import { at, message, MOCK_MESSAGES } from './mockData';
import { createMockMailApi } from './mockMailApi';
import { parseDay, parseSearchQuery, parseSize } from './mockSearch';
import type { ListQuery, Rule } from '../types';

const NOW = at('2026-11-13T09:00:00Z');

const query = (extra: Partial<ListQuery> = {}): ListQuery => ({
  accountId: 'work',
  folderId: 'work:inbox',
  role: null,
  filter: 'all',
  search: null,
  offset: 0,
  limit: 50,
  ...extra,
});

describe('mock backend: search language', () => {
  it('reads operators, phrases and negations, and explains what it cannot', () => {
    const parsed = parseSearchQuery(
      'from:boss "site visit" -is:READ has:attachments larger:2m after:2026-13-01 is:nope cc: before:yesterday',
      NOW,
    );
    expect(parsed.terms).toEqual([
      { field: 'from', value: 'boss', negated: false },
      { field: 'text', value: 'site visit', negated: false },
      { field: 'is', value: 'read', negated: true },
      { field: 'has', value: 'attachment', negated: false },
      { field: 'larger', value: '2m', negated: false },
      { field: 'before', value: 'yesterday', negated: false },
    ]);
    expect(parsed.errors).toEqual([
      'after: needs a date like 2026-10-01, today or yesterday',
      'is: can be unread, read, flagged, pinned or answered',
      'cc: needs something after it, like cc:amina',
    ]);
  });

  it('reads sizes and days', () => {
    expect(parseSize('500k')).toBe(500 * 1024);
    expect(parseSize('2m')).toBe(2 * 1024 * 1024);
    expect(parseSize('1.5MB')).toBe(Math.round(1.5 * 1024 * 1024));
    expect(parseSize('big')).toBeNull();
    expect(parseDay('2026-02-30', NOW)).toBeNull();
    const today = parseDay('today', NOW) ?? 0;
    const yesterday = parseDay('yesterday', NOW) ?? 0;
    // A day earlier (23 to 25 hours, across a clock change).
    expect(today - yesterday).toBeGreaterThanOrEqual(23 * 3600);
    expect(today - yesterday).toBeLessThanOrEqual(25 * 3600);
    expect(today).toBeLessThanOrEqual(NOW);
    expect(NOW - today).toBeLessThan(86_400);
  });

  it('filters the list with the language, as the real backend does', async () => {
    const api = createMockMailApi();
    const ids = async (search: string, extra: Partial<ListQuery> = {}) =>
      (await api.listMessages(query({ search, ...extra }))).items.map((m) => m.id);
    expect(await ids('from:wanjiru')).toEqual(['m1']);
    expect(await ids('-from:wanjiru')).toEqual(['m2', 'm5', 'm6']);
    expect(await ids('is:unread has:attachment')).toEqual(['m1']);
    expect(await ids('category:invoices')).toEqual(['m6']);
    expect(await ids('subject:"nightly build"')).toEqual(['m2']);
    expect(await ids('body:installers')).toEqual(['m2']);
    expect(await ids('larger:1m')).toEqual(['m1']);
    expect(await ids('smaller:5k')).toEqual(['m2', 'm5']);
    expect(await ids('cc:amina')).toEqual(['m1']);
    expect(await ids('in:inbox is:answered')).toEqual(['m6']);
    // A term it can't read is left out; the rest still applies.
    expect(await ids('is:nope from:peter')).toEqual(['m6']);
  });

  it('joins terms with OR, looks account-wide for in:, and leaves Trash and Spam out of wide searches', async () => {
    expect(parseSearchQuery('OR from:amina OR from:peter invoice OR', NOW).terms).toEqual([
      { field: 'from', value: 'amina', negated: false, orWithNext: true },
      { field: 'from', value: 'peter', negated: false },
      { field: 'text', value: 'invoice', negated: false },
    ]);
    const api = createMockMailApi();
    const ids = async (search: string, extra: Partial<ListQuery> = {}) =>
      (await api.listMessages(query({ search, ...extra }))).items.map((m) => m.id);
    expect(await ids('from:wanjiru OR from:peter')).toEqual(['m1', 'm6']);
    expect(await ids('is:unread from:wanjiru OR from:builds')).toEqual(['m1', 'm2']);
    // Naming a folder looks in the whole account, whatever the open folder.
    expect(await ids('in:clients')).toEqual(['m4']);
    await api.moveMessages(['m6'], { role: 'trash' });
    expect(await ids('INV-0142', { searchScope: 'account' })).toEqual(['s1']);
    expect(await ids('INV-0142 in:trash')).toEqual(['m6']);
  });

  it('searches by day in local time', async () => {
    const noon = (day: string) => at(`${day}T12:00:00Z`);
    const api = createMockMailApi({
      now: () => NOW,
      messages: [
        message({ id: 'a', accountId: 'work', folderId: 'work:inbox', subject: 'Old', date: noon('2026-10-01') }),
        message({ id: 'b', accountId: 'work', folderId: 'work:inbox', subject: 'Yesterday', date: noon('2026-11-12') }),
        message({ id: 'c', accountId: 'work', folderId: 'work:inbox', subject: 'Today', date: noon('2026-11-13') }),
      ],
    });
    const ids = async (search: string) => (await api.listMessages(query({ search }))).items.map((m) => m.id);
    expect(await ids('before:2026-11-12')).toEqual(['a']);
    expect(await ids('after:2026-11-12')).toEqual(['c', 'b']);
    expect(await ids('after:yesterday -after:today')).toEqual(['b']);
    expect((await api.parseSearch('after:today')).errors).toEqual([]);
  });

  it('looks in the folder, its subfolders, the account or everywhere', async () => {
    const api = createMockMailApi({
      messages: [
        ...MOCK_MESSAGES,
        message({ id: 'acme', accountId: 'work', folderId: 'work:Clients/Acme', subject: 'Logo for Acme' }),
      ],
    });
    const ids = async (extra: Partial<ListQuery>) =>
      (await api.listMessages(query({ folderId: 'work:Clients', search: 'logo', ...extra }))).items.map((m) => m.id).sort();
    expect(await ids({})).toEqual(['m4']);
    expect(await ids({ searchScope: 'subfolders' })).toEqual(['acme', 'm4']);
    expect(await ids({ searchScope: 'account', search: 'lunch' })).toEqual([]);
    expect(await ids({ searchScope: 'all', search: 'lunch' })).toEqual(['d1', 'm3']);
    // Unified views: the role, or every folder of every account.
    const unified = { accountId: null, folderId: null, role: 'inbox' as const };
    expect(await ids({ ...unified, search: 'invoice' })).toEqual(['m6']);
    expect(await ids({ ...unified, search: 'invoice', searchScope: 'all' })).toEqual(['m6', 's1']);
    // Without a search the scope changes nothing.
    expect((await api.listMessages(query({ searchScope: 'all' }))).total).toBe(4);
  });
});

const rule = (patch: Partial<Rule> & Pick<Rule, 'id' | 'name'>): Rule => ({
  enabled: true,
  accountId: 'work',
  match: 'all',
  conditions: [],
  exceptions: [],
  actions: [{ kind: 'flag' }],
  stopProcessing: false,
  keepOnServer: false,
  serverStatus: 'local',
  serverNote: null,
  order: 0,
  ...patch,
});

describe('mock backend: rules', () => {
  it('validates, stores in order, and says where a rule runs', async () => {
    const api = createMockMailApi();
    const base = { ...rule({ id: 'x', name: 'Boss' }), id: null };
    await expect(api.saveRule({ ...base, name: ' ' })).rejects.toMatchObject({ message: 'Give the rule a name.' });
    await expect(api.saveRule({ ...base, actions: [] })).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.saveRule({ ...base, conditions: [{ kind: 'from', value: ' , ' }] })).rejects.toMatchObject({
      message: 'Condition 1: type the words or addresses to look for.',
    });
    await expect(
      api.saveRule({ ...base, accountId: null, actions: [{ kind: 'move', folderId: 'work:Clients' }] }),
    ).rejects.toMatchObject({ message: expect.stringContaining('every account has') as unknown });
    await expect(api.saveRule({ ...base, actions: [{ kind: 'move', folderId: 'personal:Travel' }] })).rejects.toMatchObject({
      kind: 'invalid',
    });

    const changed = vi.fn();
    api.on('mail://rules-changed', changed);
    // Saved here at once; the server copy follows, announced by mail://rules-changed.
    let rules = await api.saveRule({ ...base, keepOnServer: true, conditions: [{ kind: 'from', value: 'wanjiru' }] });
    expect(rules[0]).toMatchObject({ name: 'Boss', serverStatus: 'local', serverNote: 'Being saved to the mail server.', order: 0 });
    await vi.waitFor(() => {
      expect(changed).toHaveBeenCalledTimes(1);
    });
    expect((await api.listRules())[0]).toMatchObject({ serverStatus: 'server', serverNote: null });
    rules = await api.saveRule({ ...base, name: 'Tag', keepOnServer: true, actions: [{ kind: 'categorize', category: 'Clients' }] });
    expect(rules[1]).toMatchObject({ serverStatus: 'local', order: 1 });
    await vi.waitFor(async () => {
      expect((await api.listRules())[1]).toMatchObject({ serverStatus: 'unsupported' });
    });
    rules = await api.listRules();
    expect(rules[1]?.serverNote).toMatch(/this device only/);
    expect(await api.saveRule({ ...base, name: 'Here', keepOnServer: false })).toContainEqual(
      expect.objectContaining({ name: 'Here', serverStatus: 'local', serverNote: null }) as unknown,
    );
    await api.deleteRule((await api.listRules())[2]?.id ?? '');
    const [first, second] = rules.map((r) => r.id);
    if (!first || !second) throw new Error('two rules expected');
    await expect(api.reorderRules([second])).rejects.toMatchObject({ kind: 'invalid' });
    expect((await api.reorderRules([second, first])).map((r) => [r.name, r.order])).toEqual([
      ['Tag', 0],
      ['Boss', 1],
    ]);
    expect((await api.deleteRule(second)).map((r) => [r.name, r.order])).toEqual([['Boss', 0]]);
  });

  it('previews and runs rules on a folder, in order, stopping when asked', async () => {
    const api = createMockMailApi({
      rules: [
        rule({
          id: 'clients',
          name: 'Clients',
          match: 'any',
          conditions: [
            { kind: 'from', value: 'wanjiru, peter@client.example' },
            { kind: 'importance', value: 'high' },
          ],
          exceptions: [{ kind: 'category', value: 'Invoices' }],
          actions: [{ kind: 'move', folderId: 'work:Clients' }, { kind: 'markRead' }],
          stopProcessing: true,
        }),
        rule({ id: 'flag', name: 'Flag everything', actions: [{ kind: 'flag' }] }),
        rule({ id: 'off', name: 'Off', enabled: false, actions: [{ kind: 'delete' }] }),
      ],
    });
    const [clients] = await api.listRules();
    if (!clients) throw new Error('rule expected');
    // m1 (from Wanjiru); m6 is from Peter but has the Invoices category.
    expect(await api.previewRule({ ...clients, id: null }, 'work:inbox')).toBe(1);
    expect(await api.previewRule({ ...clients, id: null, exceptions: [] }, 'work:inbox')).toBe(2);
    expect(await api.runRules(null, 'work:inbox')).toEqual({ checked: 4, matched: 4 });
    expect(await api.getMessage('m1', false)).toMatchObject({ folderId: 'work:Clients', unread: false, flagged: false });
    expect(await api.getMessage('m2', false)).toMatchObject({ folderId: 'work:inbox', flagged: true });
    // Only the named rule; a disabled rule runs when named.
    expect(await api.runRules(['off'], 'work:inbox')).toEqual({ checked: 3, matched: 3 });
    expect((await api.listMessages(query({ folderId: 'work:trash' }))).total).toBe(3);
  });
});

describe('mock backend: trusted picture senders', () => {
  it('loads pictures from trusted senders and domains', async () => {
    const api = createMockMailApi();
    expect(await api.getMessage('m2', false)).toMatchObject({ blockedRemoteImages: 1 });
    await expect(api.trustImageSender('nobody', false)).rejects.toMatchObject({ kind: 'invalid' });
    expect(await api.trustImageSender('Builds@CI.example', true)).toEqual([{ sender: 'ci.example', wholeDomain: true }]);
    const shown = await api.getMessage('m2', false);
    expect(shown).toMatchObject({ blockedRemoteImages: 0, imagesTrusted: true });
    expect(shown.html).toContain('badge.svg');
    // Messages without pictures to load aren't marked.
    expect((await api.getMessage('m1', false)).imagesTrusted).toBeUndefined();
    expect(await api.untrustImageSender('ci.example')).toEqual([]);
    expect(await api.getMessage('m2', false)).toMatchObject({ blockedRemoteImages: 1 });
    await api.trustImageSender('builds@ci.example', false);
    expect(await api.listImageSenders()).toEqual([{ sender: 'builds@ci.example', wholeDomain: false }]);
    expect((await api.getMessage('m2', false)).imagesTrusted).toBe(true);
  });
});
