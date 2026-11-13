import { MOCK_MESSAGES } from '../../api/mockData';
import {
  avatarColour,
  emptyCopy,
  filterMessages,
  initials,
  isFiltered,
  listMeta,
  NO_FILTERS,
  resolveSelection,
} from './selectors';

const inbox = MOCK_MESSAGES.filter((m) => m.folder === 'inbox');
const ids = (list: readonly { id: string }[]) => list.map((m) => m.id);

describe('filterMessages', () => {
  it('returns everything with no filters', () => {
    expect(filterMessages(inbox, NO_FILTERS)).toHaveLength(inbox.length);
  });

  it('filters by account', () => {
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, accountId: 'personal' }))).toEqual(['m3', 'm7']);
  });

  it('filters by label', () => {
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, labelId: 'clients' }))).toEqual(['m1', 'm4']);
  });

  it('filters unread and flagged', () => {
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, read: 'unread' }))).toEqual(['m1', 'm2', 'm3', 'm5']);
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, read: 'flagged' }))).toEqual(['m4']);
  });

  it('searches subject, preview and sender, ignoring case and spaces', () => {
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, query: '  NAIVASHA ' }))).toEqual(['m7']);
    expect(ids(filterMessages(inbox, { ...NO_FILTERS, query: 'fitout.example' }))).toEqual(['m1']);
  });

  it('keeps the open message visible under read filters only', () => {
    const read = { ...NO_FILTERS, read: 'unread' as const };
    expect(ids(filterMessages(inbox, read, 'm4'))).toEqual(['m1', 'm2', 'm3', 'm4', 'm5']);
    expect(ids(filterMessages(inbox, { ...read, accountId: 'personal' }, 'm4'))).toEqual(['m3']);
  });

  it('combines filters', () => {
    expect(filterMessages(inbox, { ...NO_FILTERS, accountId: 'personal', read: 'flagged' })).toEqual([]);
  });
});

describe('resolveSelection', () => {
  it('keeps a visible selection and falls back to the first message', () => {
    expect(resolveSelection(inbox, 'm4')).toBe('m4');
    expect(resolveSelection(inbox, 'gone')).toBe('m1');
    expect(resolveSelection(inbox, null)).toBe('m1');
    expect(resolveSelection([], 'm1')).toBeNull();
  });
});

describe('initials', () => {
  it.each([
    ['Wanjiru Kamau', 'WK'],
    ['Calendar', 'C'],
    ['builds@ci.example', 'B'],
    ['  amina   hassan  ', 'AH'],
    ['Ólafur Ragnar Grímsson', 'ÓR'],
    ['', '?'],
    ['---', '?'],
  ])('%s gives %s', (name, expected) => {
    expect(initials(name)).toBe(expected);
  });
});

describe('avatarColour', () => {
  it('is stable and case-insensitive', () => {
    expect(avatarColour('A@b.example')).toBe(avatarColour('a@B.example'));
    expect(avatarColour('x')).toMatch(/^#[0-9A-F]{6}$/);
  });
});

describe('copy helpers', () => {
  it('describes the list', () => {
    expect(listMeta(1, 'Work')).toBe('1 message, Work');
    expect(listMeta(8, null)).toBe('8 messages, All accounts');
  });

  it('explains empty folders and empty filters', () => {
    expect(emptyCopy('spam', false).title).toBe('No spam');
    expect(emptyCopy('spam', true).title).toBe('Nothing here');
  });

  it('knows when filters are active', () => {
    expect(isFiltered(NO_FILTERS)).toBe(false);
    expect(isFiltered({ ...NO_FILTERS, query: ' ' })).toBe(false);
    expect(isFiltered({ ...NO_FILTERS, read: 'unread' })).toBe(true);
  });
});
