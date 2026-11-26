import type { Folder, MessageListItem } from '../../api/types';
import {
  applyOverrides,
  avatarColour,
  emptyCopy,
  initials,
  neighbourAfterRemoval,
  nextOverride,
  resolveSelection,
  withPinned,
  sortFolders,
  unifiedCounts,
  viewInfo,
  viewQuery,
} from './selectors';
import { MOCK_ACCOUNTS } from '../../api/mock/mockData';

const folder = (id: string, role: Folder['role'], name: string, unread = 0, total = 0): Folder => ({
  id, accountId: 'work', name, path: name, role, unread, total, delimiter: '/', favorite: false,
});

function item(id: string, extra: Partial<MessageListItem> = {}): MessageListItem {
  return {
    id, accountId: 'work', folderId: 'work:inbox', from: null, to: [], subject: id, preview: '', date: 0,
    unread: false, flagged: false, answered: false, hasAttachments: false, pinned: false, importance: 'normal',
    categories: [], flagDue: null, size: 0, ...extra,
  };
}

describe('folders', () => {
  it('puts special folders first in a fixed order, then user folders by name', () => {
    const sorted = sortFolders([
      folder('z', 'other', 'zeta'), folder('t', 'trash', 'Bin'), folder('a', 'other', 'Alpha'),
      folder('i', 'inbox', 'INBOX'), folder('s', 'sent', 'Sent Items'),
    ]);
    expect(sorted.map((f) => f.id)).toEqual(['i', 's', 't', 'a', 'z']);
  });

  it('sums unified counts across accounts', () => {
    expect(
      unifiedCounts([
        folder('a', 'inbox', 'Inbox', 3), folder('b', 'inbox', 'Inbox', 2),
        folder('c', 'drafts', 'Drafts', 0, 4), folder('d', 'other', 'X', 9, 9),
      ]),
    ).toEqual({ inboxUnread: 5, drafts: 4 });
  });
});

describe('views', () => {
  it('builds queries for unified roles and folders, none for the Outbox', () => {
    expect(viewQuery({ type: 'unified', role: 'inbox' }, 'unread', '  ')).toEqual({
      accountId: null, folderId: null, role: 'inbox', filter: 'unread', search: null,
    });
    expect(viewQuery({ type: 'folder', accountId: 'work', folderId: 'f' }, 'all', ' naivasha ')).toEqual({
      accountId: 'work', folderId: 'f', role: null, filter: 'all', search: 'naivasha',
    });
    expect(viewQuery({ type: 'outbox' }, 'all', '')).toBeNull();
  });

  it('names views', () => {
    const folders = [folder('work:Clients', 'other', 'Clients'), folder('work:junk', 'junk', 'Bulk Mail')];
    expect(viewInfo({ type: 'unified', role: 'sent' }, MOCK_ACCOUNTS, folders).title).toBe('Sent');
    const f = viewInfo({ type: 'folder', accountId: 'work', folderId: 'work:Clients' }, MOCK_ACCOUNTS, folders);
    expect(f).toEqual({ title: 'Clients', subtitle: 'Work', role: 'other' });
    expect(viewInfo({ type: 'folder', accountId: 'work', folderId: 'work:junk' }, MOCK_ACCOUNTS, folders).title).toBe('Spam');
  });
});

describe('selection', () => {
  const items = [item('a'), item('b'), item('c')];
  it('starts on the first message and keeps a visible selection', () => {
    expect(resolveSelection(items, null)).toBe('a');
    expect(resolveSelection(items, 'b')).toBe('b');
    expect(resolveSelection([], null)).toBeNull();
  });

  it('does not jump to the first message when the chosen one is gone', () => {
    expect(resolveSelection(items, 'x')).toBeNull();
    expect(resolveSelection([], 'a')).toBeNull();
  });

  it('follows a conversation whose row was renamed by a newer reply', () => {
    const grown = [item('a'), item('b2', { threadCount: 2, threadIds: ['b2', 'b'] }), item('c')];
    expect(resolveSelection(grown, 'b')).toBe('b2');
    expect(resolveSelection(grown, 'b2')).toBe('b2');
  });

  it('moves to the next message after a removal, else the previous', () => {
    expect(neighbourAfterRemoval(items, 'b')).toBe('c');
    expect(neighbourAfterRemoval(items, 'c')).toBe('b');
    expect(neighbourAfterRemoval([item('a')], 'a')).toBeNull();
    expect(neighbourAfterRemoval(items, 'x')).toBeNull();
  });
});

describe('withPinned', () => {
  it('puts the opened message back in date order, once', () => {
    const list = [item('a', { date: 30 }), item('c', { date: 10 })];
    expect(withPinned(list, item('b', { date: 20 })).map((m) => m.id)).toEqual(['a', 'b', 'c']);
    expect(withPinned(list, item('z', { date: 1 })).map((m) => m.id)).toEqual(['a', 'c', 'z']);
    expect(withPinned(list, item('a', { date: 30 })).map((m) => m.id)).toEqual(['a', 'c']);
    expect(withPinned(list, null)).toEqual(list);
  });
});

describe('applyOverrides', () => {
  const items = [item('a', { unread: true }), item('b')];
  it('applies pending changes and hides removed messages', () => {
    const out = applyOverrides(items, { a: { unread: false, version: 1, settled: false }, b: { removed: true, version: 1, settled: false } }, 5);
    expect(out).toEqual([item('a', { unread: false })]);
  });

  it('drops settled changes once newer data arrives', () => {
    const overrides = { a: { unread: false, version: 1, settled: true } };
    expect(applyOverrides(items, overrides, 1)[0]?.unread).toBe(false);
    expect(applyOverrides(items, overrides, 2)[0]?.unread).toBe(true);
  });
});

describe('nextOverride', () => {
  it('builds on a change that is still pending', () => {
    const pending = { unread: false, version: 3, settled: false };
    expect(nextOverride(pending, { flagged: true }, 3)).toEqual({ unread: false, flagged: true, version: 3, settled: false });
    // Settled, but the list has not been reloaded since: it still applies.
    expect(nextOverride({ ...pending, settled: true }, { flagged: true }, 3).unread).toBe(false);
  });

  it('does not bring back an archive the list has already caught up with', () => {
    const archived = { removed: true, version: 3, settled: true };
    const flagged = nextOverride(archived, { flagged: true }, 5);
    expect(flagged).toEqual({ flagged: true, version: 5, settled: false });
    // The message, listed again, stays visible while the flag is saved.
    expect(applyOverrides([item('a')], { a: flagged }, 5)).toEqual([item('a', { flagged: true })]);
  });

  it('starts fresh for a message with no change', () => {
    expect(nextOverride(undefined, { removed: true }, 1)).toEqual({ removed: true, version: 1, settled: false });
  });
});

describe('display helpers', () => {
  it.each([
    ['Wanjiru Kamau', 'WK'], ['builds@ci.example', 'B'], ['Ólafur Ragnar', 'ÓR'], ['', '?'], ['---', '?'],
  ])('initials of %s is %s', (name, expected) => {
    expect(initials(name)).toBe(expected);
  });

  it('gives stable avatar colours', () => {
    expect(avatarColour('A@b.example')).toBe(avatarColour('a@B.example'));
  });

  it('explains empty views', () => {
    expect(emptyCopy('junk', false).title).toBe('No spam');
    expect(emptyCopy('junk', true).title).toBe('Nothing here');
    expect(emptyCopy('outbox', false).title).toBe('Outbox is empty');
  });
});
