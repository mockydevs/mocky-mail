import type { Folder } from '../../api/types';
import { folderLabel, sortFolders } from './selectors';

export interface FolderNode {
  /** Stable across sessions: account id plus the raw server path. */
  key: string;
  /** The folder's decoded name, or the raw path segment for a container. */
  label: string;
  /** null for a server container that is not itself a folder. */
  folder: Folder | null;
  children: FolderNode[];
}

const ROLE_RANK = ['inbox', 'drafts', 'sent', 'archive', 'flagged', 'all', 'junk', 'trash', 'other'] as const;

function rank(node: FolderNode): number {
  return ROLE_RANK.indexOf(node.folder?.role ?? 'other');
}

function sortNodes(nodes: FolderNode[]): FolderNode[] {
  nodes.sort((a, b) => rank(a) - rank(b) || a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }));
  for (const node of nodes) sortNodes(node.children);
  return nodes;
}

/**
 * Nest one account's folders by splitting `path` on each folder's delimiter.
 * Special folders come first at every level, then the rest by name. A parent
 * path that is not a folder becomes a label-only node.
 */
export function buildFolderTree(folders: readonly Folder[]): FolderNode[] {
  const roots: FolderNode[] = [];
  const byPath = new Map<string, FolderNode>();
  for (const folder of sortFolders(folders)) {
    const segments = (folder.delimiter ? folder.path.split(folder.delimiter) : [folder.path]).filter((s) => s !== '');
    if (segments.length === 0) segments.push(folder.path);
    let siblings = roots;
    let prefix = '';
    segments.forEach((segment, i) => {
      prefix = i === 0 ? segment : `${prefix}${folder.delimiter ?? ''}${segment}`;
      let node = byPath.get(prefix);
      if (!node) {
        node = { key: `${folder.accountId}:${prefix}`, label: segment, folder: null, children: [] };
        byPath.set(prefix, node);
        siblings.push(node);
      }
      if (i === segments.length - 1) {
        node.folder = folder;
        node.label = folderLabel(folder);
      }
      siblings = node.children;
    });
  }
  return sortNodes(roots);
}

export const COLLAPSED_STORAGE_KEY = 'mocky-mail.folders.collapsed';

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

/** Collapsed node keys. Never throws: storage can be blocked, full or corrupt. */
export function readCollapsed(storage: StorageLike | null): Set<string> {
  try {
    const raw = storage?.getItem(COLLAPSED_STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((k): k is string => typeof k === 'string') : []);
  } catch {
    return new Set();
  }
}

export function writeCollapsed(storage: StorageLike | null, keys: ReadonlySet<string>): boolean {
  try {
    if (!storage) return false;
    storage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify([...keys].sort()));
    return true;
  } catch {
    return false;
  }
}

export function safeLocalStorage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    return null;
  }
}
