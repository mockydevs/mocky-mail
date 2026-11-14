import type { Folder } from '../../api/types';
import { folderLabel, sortFolders } from './selectors';

export interface FolderNode {
  /** Stable across sessions: account id plus the raw server path. */
  key: string;
  /** The folder's decoded name, or the decoded path segment for a container. */
  label: string;
  /** null for a server container that is not itself a folder. */
  folder: Folder | null;
  children: FolderNode[];
}

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,';

/**
 * Decode an IMAP folder-name segment from modified UTF-7 (RFC 3501 5.1.3),
 * the same way the backend decodes folder names. Returns the input unchanged
 * when it isn't valid, so a label is never lost.
 */
export function decodeImapUtf7(encoded: string): string {
  let out = '';
  let rest = encoded;
  for (let start = rest.indexOf('&'); start !== -1; start = rest.indexOf('&')) {
    out += rest.slice(0, start);
    const end = rest.indexOf('-', start + 1);
    if (end === -1) return encoded;
    const chunk = rest.slice(start + 1, end);
    if (chunk === '') {
      out += '&';
    } else {
      let bits = 0;
      let count = 0;
      const units: number[] = [];
      for (const c of chunk) {
        const value = BASE64.indexOf(c);
        if (value === -1) return encoded;
        bits = ((bits << 6) | value) & 0xffffff;
        count += 6;
        if (count >= 16) {
          count -= 16;
          units.push((bits >> count) & 0xffff);
        }
      }
      if (units.length === 0) return encoded;
      out += String.fromCharCode(...units);
    }
    rest = rest.slice(end + 1);
  }
  return out + rest;
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
        node = { key: `${folder.accountId}:${prefix}`, label: decodeImapUtf7(segment), folder: null, children: [] };
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
