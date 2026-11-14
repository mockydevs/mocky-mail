import type { Account, Folder, FolderRole, MessageListItem, ReadFilter } from '../../api/types';
import type { PageQuery } from './useMessagePage';

/** What the message list shows. */
export type MailView =
  | { type: 'unified'; role: UnifiedRole }
  | { type: 'folder'; accountId: string; folderId: string }
  | { type: 'outbox' };

export const UNIFIED_ROLES = ['inbox', 'drafts', 'sent'] as const;
export type UnifiedRole = (typeof UNIFIED_ROLES)[number];

export const INBOX_VIEW: MailView = { type: 'unified', role: 'inbox' };

const ROLE_ORDER: readonly FolderRole[] = ['inbox', 'drafts', 'sent', 'archive', 'flagged', 'all', 'junk', 'trash', 'other'];

export const ROLE_TITLES: Readonly<Record<FolderRole, string>> = {
  inbox: 'Inbox',
  drafts: 'Drafts',
  sent: 'Sent',
  archive: 'Archive',
  flagged: 'Flagged',
  all: 'All mail',
  junk: 'Spam',
  trash: 'Trash',
  other: 'Folder',
};

export function sameView(a: MailView, b: MailView): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Special folders first in a fixed order, then the user's folders by name. */
export function sortFolders(folders: readonly Folder[]): Folder[] {
  return [...folders].sort((a, b) => {
    const byRole = ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role);
    return byRole !== 0 ? byRole : a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
  });
}

/** The list query for a view, or null for the Outbox (which is not a folder). */
export function viewQuery(view: MailView, filter: ReadFilter, search: string): PageQuery | null {
  const text = search.trim() === '' ? null : search.trim();
  switch (view.type) {
    case 'outbox':
      return null;
    case 'unified':
      return { accountId: null, folderId: null, role: view.role, filter, search: text };
    case 'folder':
      return { accountId: view.accountId, folderId: view.folderId, role: null, filter, search: text };
  }
}

export interface ViewInfo {
  title: string;
  subtitle: string;
  role: FolderRole | null;
}

export function viewInfo(view: MailView, accounts: readonly Account[], folders: readonly Folder[]): ViewInfo {
  switch (view.type) {
    case 'outbox':
      return { title: 'Outbox', subtitle: 'All accounts', role: null };
    case 'unified':
      return { title: ROLE_TITLES[view.role], subtitle: 'All accounts', role: view.role };
    case 'folder': {
      const folder = folders.find((f) => f.id === view.folderId);
      const account = accounts.find((a) => a.id === view.accountId);
      return {
        title: folder ? (folder.role === 'other' ? folder.name : ROLE_TITLES[folder.role]) : 'Folder',
        subtitle: account?.displayName ?? '',
        role: folder?.role ?? null,
      };
    }
  }
}

export function folderLabel(folder: Folder): string {
  return folder.role === 'other' ? folder.name : ROLE_TITLES[folder.role];
}

export interface UnifiedCounts {
  inboxUnread: number;
  drafts: number;
}

export function unifiedCounts(folders: readonly Folder[]): UnifiedCounts {
  let inboxUnread = 0;
  let drafts = 0;
  for (const f of folders) {
    if (f.role === 'inbox') inboxUnread += f.unread;
    if (f.role === 'drafts') drafts += f.total;
  }
  return { inboxUnread, drafts };
}

/** Keep the chosen message if it is still visible, otherwise the first one. */
export function resolveSelection(items: readonly MessageListItem[], selectedId: string | null): string | null {
  if (selectedId !== null && items.some((m) => m.id === selectedId)) return selectedId;
  return items[0]?.id ?? null;
}

/**
 * Keep the message the user opened in the list while a filter or search is
 * active, even after reading it takes it out of the server's results.
 * It goes back in date order.
 */
export function withPinned(items: readonly MessageListItem[], pinned: MessageListItem | null): MessageListItem[] {
  if (!pinned || items.some((m) => m.id === pinned.id)) return [...items];
  const at = items.findIndex((m) => (m.date ?? 0) < (pinned.date ?? 0));
  return at < 0 ? [...items, pinned] : [...items.slice(0, at), pinned, ...items.slice(at)];
}

/** The message to show after `removedId` leaves the list: the next one, else the previous. */
export function neighbourAfterRemoval(items: readonly MessageListItem[], removedId: string): string | null {
  const index = items.findIndex((m) => m.id === removedId);
  if (index < 0) return null;
  return items[index + 1]?.id ?? items[index - 1]?.id ?? null;
}

export interface Override {
  unread?: boolean;
  flagged?: boolean;
  removed?: boolean;
  /** Page version when the change was made. */
  version: number;
  /** The backend confirmed it; drop once newer data arrives. */
  settled: boolean;
}

/**
 * Apply optimistic changes. A settled change only applies until the next
 * page version, which already contains it.
 */
export function applyOverrides(
  items: readonly MessageListItem[],
  overrides: Readonly<Record<string, Override>>,
  version: number,
): MessageListItem[] {
  const out: MessageListItem[] = [];
  for (const item of items) {
    const o = overrides[item.id];
    if (!o || (o.settled && version > o.version)) {
      out.push(item);
      continue;
    }
    if (o.removed) continue;
    out.push({
      ...item,
      ...(o.unread !== undefined ? { unread: o.unread } : {}),
      ...(o.flagged !== undefined ? { flagged: o.flagged } : {}),
    });
  }
  return out;
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

export interface EmptyCopy {
  title: string;
  text: string;
}

export function emptyCopy(role: FolderRole | 'outbox' | null, filtered: boolean): EmptyCopy {
  if (filtered) return { title: 'Nothing here', text: 'No messages match this search or filter.' };
  switch (role) {
    case 'outbox':
      return { title: 'Outbox is empty', text: 'Everything you wrote has been sent.' };
    case 'archive':
      return { title: 'Archive is empty', text: 'Archived mail stays searchable here.' };
    case 'junk':
      return { title: 'No spam', text: 'Suspicious mail lands here.' };
    case 'trash':
      return { title: 'Trash is empty', text: 'Deleted mail stays here until you empty it.' };
    case 'drafts':
      return { title: 'No drafts', text: 'Messages you save without sending are kept here.' };
    case 'inbox':
      return { title: 'All caught up', text: 'New messages will show up here.' };
    case 'sent':
    case 'all':
    case 'flagged':
    case 'other':
    case null:
      return { title: 'Nothing here yet', text: 'Messages in this folder will show up here.' };
  }
}
