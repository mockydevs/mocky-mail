import type { FolderId, MessageSummary } from '../../api/types';

export type ReadFilter = 'all' | 'unread' | 'flagged';

export interface ListFilters {
  accountId: string | null;
  labelId: string | null;
  read: ReadFilter;
  query: string;
}

export const NO_FILTERS: ListFilters = { accountId: null, labelId: null, read: 'all', query: '' };

function matchesQuery(m: MessageSummary, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  return [m.subject, m.preview, m.counterpart.name, m.counterpart.address].some((field) =>
    field.toLocaleLowerCase().includes(needle),
  );
}

/**
 * Messages visible in the list after account, label, read-state and search
 * filters. `openId` stays visible under Unread/Flagged even after it is read
 * or unflagged, so the message the user is reading does not vanish.
 */
export function filterMessages(
  messages: readonly MessageSummary[],
  filters: ListFilters,
  openId: string | null = null,
): MessageSummary[] {
  return messages.filter(
    (m) =>
      (filters.accountId === null || m.accountId === filters.accountId) &&
      (filters.labelId === null || m.labelIds.includes(filters.labelId)) &&
      (m.id === openId || filters.read !== 'unread' || m.unread) &&
      (m.id === openId || filters.read !== 'flagged' || m.flagged) &&
      matchesQuery(m, filters.query),
  );
}

/** Keep the chosen message if it is still visible, otherwise the first one. */
export function resolveSelection(visible: readonly MessageSummary[], selectedId: string | null): string | null {
  if (selectedId !== null && visible.some((m) => m.id === selectedId)) return selectedId;
  return visible[0]?.id ?? null;
}

/** "WK" for "Wanjiru Kamau", "B" for "builds@ci.example". */
export function initials(name: string): string {
  const words = name
    .replace(/@.*/, '')
    .split(/[\s._-]+/)
    .filter((w) => /\p{L}|\p{N}/u.test(w));
  const letters = words.slice(0, 2).map((w) => Array.from(w)[0] ?? '');
  return letters.join('').toLocaleUpperCase() || '?';
}

const AVATAR_COLOURS = ['#2450C8', '#0F7B6C', '#8A3FB0', '#B5471B', '#4B5563', '#1E6B3A', '#A1356B'] as const;

/** Stable avatar colour per address. All colours pass 4.5:1 with white text. */
export function avatarColour(seed: string): string {
  let hash = 0;
  for (const ch of seed.toLocaleLowerCase()) {
    hash = (hash * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  }
  return AVATAR_COLOURS[hash % AVATAR_COLOURS.length] ?? AVATAR_COLOURS[0];
}

export const FOLDER_TITLES: Readonly<Record<FolderId, string>> = {
  inbox: 'Inbox',
  outbox: 'Outbox',
  drafts: 'Drafts',
  sent: 'Sent',
  archive: 'Archive',
  spam: 'Spam',
  trash: 'Trash',
};

export function listMeta(count: number, accountLabel: string | null): string {
  return `${count} ${count === 1 ? 'message' : 'messages'}, ${accountLabel ?? 'All accounts'}`;
}

export interface EmptyCopy {
  title: string;
  text: string;
}

export function emptyCopy(folder: FolderId, filtered: boolean): EmptyCopy {
  if (filtered) return { title: 'Nothing here', text: 'No messages match these filters.' };
  switch (folder) {
    case 'archive':
      return { title: 'Archive is empty', text: 'Archived mail stays searchable here.' };
    case 'spam':
      return { title: 'No spam', text: 'Suspicious mail lands here and is deleted after 30 days.' };
    case 'trash':
      return { title: 'Trash is empty', text: 'Deleted mail stays here for 30 days.' };
    case 'outbox':
      return { title: 'Outbox is empty', text: 'Everything you wrote has been sent.' };
    case 'drafts':
      return { title: 'No drafts', text: 'Messages you start but do not send are kept here.' };
    case 'inbox':
    case 'sent':
      return { title: 'Nothing here yet', text: 'New messages will show up here.' };
  }
}

export function isFiltered(filters: ListFilters): boolean {
  return filters.accountId !== null || filters.labelId !== null || filters.read !== 'all' || filters.query.trim() !== '';
}
