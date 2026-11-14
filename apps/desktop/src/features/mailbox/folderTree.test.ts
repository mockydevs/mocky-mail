import type { Folder } from '../../api/types';
import {
  buildFolderTree,
  COLLAPSED_STORAGE_KEY,
  decodeImapUtf7,
  readCollapsed,
  writeCollapsed,
  type FolderNode,
} from './folderTree';

function folder(path: string, name: string, role: Folder['role'] = 'other', delimiter: string | null = '/'): Folder {
  return { id: `a:${path}`, accountId: 'a', name, path, role, unread: 0, total: 0, delimiter };
}

/** "label(child,child)" for compact assertions; containers are marked with *. */
function shape(nodes: FolderNode[]): string {
  return nodes
    .map((n) => `${n.folder ? '' : '*'}${n.label}${n.children.length ? `(${shape(n.children)})` : ''}`)
    .join(',');
}

describe('buildFolderTree', () => {
  it('nests on "/" with special folders first and leaf names as labels', () => {
    const tree = buildFolderTree([
      folder('Zeta', 'Zeta'),
      folder('Clients/Acme', 'Acme'),
      folder('Clients', 'Clients'),
      folder('Trash', 'Bin', 'trash'),
      folder('INBOX', 'INBOX', 'inbox'),
      folder('INBOX/Receipts', 'Receipts'),
      folder('Sent Items', 'Sent Items', 'sent'),
    ]);
    expect(shape(tree)).toBe('Inbox(Receipts),Sent,Trash,Clients(Acme),Zeta');
  });

  it('nests on "." too', () => {
    const tree = buildFolderTree([
      folder('INBOX', 'INBOX', 'inbox', '.'),
      folder('INBOX.Travel', 'Travel', 'other', '.'),
      folder('INBOX.Travel.Kenya', 'Kenya', 'other', '.'),
    ]);
    expect(shape(tree)).toBe('Inbox(Travel(Kenya))');
  });

  it('shows a missing parent as a label-only node with the raw segment', () => {
    const tree = buildFolderTree([folder('Projects/2026', '2026'), folder('Projects/2025', '2025')]);
    expect(shape(tree)).toBe('*Projects(2025,2026)');
    expect(tree[0]?.folder).toBeNull();
    expect(tree[0]?.key).toBe('a:Projects');
  });

  it('keeps decoded names for display, never raw modified UTF-7 paths', () => {
    expect(shape(buildFolderTree([folder('R&AOk-sum&AOk-s', 'Résumés')]))).toBe('Résumés');
  });

  it('decodes a container that only exists as a parent path', () => {
    const tree = buildFolderTree([folder('&BB4EQgRHBDUEQgRL-/2026', '2026'), folder('Caf&AOk-/Menu', 'Menu')]);
    expect(shape(tree)).toBe('*Café(Menu),*Отчеты(2026)');
    // The key keeps the raw path so collapsed state survives.
    expect(tree.find((n) => n.label === 'Café')?.key).toBe('a:Caf&AOk-');
  });

  it('treats a folder without a delimiter as flat, and ignores empty segments', () => {
    expect(shape(buildFolderTree([folder('A/B', 'A/B', 'other', null)]))).toBe('A/B');
    expect(shape(buildFolderTree([folder('/Lead/', 'Lead')]))).toBe('Lead');
  });
});

describe('decodeImapUtf7', () => {
  it('decodes accents, non-Latin scripts, emoji and a literal ampersand', () => {
    expect(decodeImapUtf7('R&AOk-sum&AOk-s')).toBe('Résumés');
    expect(decodeImapUtf7('&ZeVnLIqe-')).toBe('日本語');
    expect(decodeImapUtf7('&2D3eAA-')).toBe('\u{1F600}');
    expect(decodeImapUtf7('Tom &- Jerry')).toBe('Tom & Jerry');
    expect(decodeImapUtf7('Plain')).toBe('Plain');
  });

  it('leaves malformed names as they are', () => {
    for (const bad of ['&AOk', '&!!!-', '&A-']) expect(decodeImapUtf7(bad)).toBe(bad);
  });
});

describe('collapsed state storage', () => {
  const memory = (initial: Record<string, string> = {}) => {
    const data = new Map(Object.entries(initial));
    return {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => {
        data.set(k, v);
      },
    };
  };
  const blocked = {
    getItem: () => {
      throw new Error('SecurityError');
    },
    setItem: () => {
      throw new Error('QuotaExceededError');
    },
  };

  it('round-trips keys and ignores junk', () => {
    const storage = memory();
    expect(writeCollapsed(storage, new Set(['a:Clients']))).toBe(true);
    expect([...readCollapsed(storage)]).toEqual(['a:Clients']);
    expect(readCollapsed(memory({ [COLLAPSED_STORAGE_KEY]: '{not json' })).size).toBe(0);
    expect([...readCollapsed(memory({ [COLLAPSED_STORAGE_KEY]: '["x", 3, null]' }))]).toEqual(['x']);
  });

  it('never throws when storage is blocked', () => {
    expect(readCollapsed(blocked).size).toBe(0);
    expect(writeCollapsed(blocked, new Set(['x']))).toBe(false);
    expect(writeCollapsed(null, new Set())).toBe(false);
  });
});
