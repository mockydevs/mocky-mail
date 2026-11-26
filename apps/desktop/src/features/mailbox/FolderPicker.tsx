import { useId, useState, type CSSProperties } from 'react';

import type { Account, Folder, FolderRole, MessageListItem, MoveTarget } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { Modal } from '../../ui/Modal';
import { buildFolderTree, type FolderNode } from './folderTree';
import { ROLE_TITLES } from './selectors';

/** Where messages from several accounts can go: folders every account has. */
export const SHARED_ROLES: readonly FolderRole[] = ['inbox', 'archive', 'trash'];

const ICONS: Partial<Record<FolderRole, IconName>> = {
  inbox: 'inbox',
  drafts: 'drafts',
  sent: 'sent',
  archive: 'archive',
  junk: 'junk',
  trash: 'trash',
};

interface Props {
  mode: 'move' | 'copy';
  rows: readonly MessageListItem[];
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  onPick: (target: MoveTarget, name: string) => void;
  onClose: () => void;
}

/** Folders (and containers holding matches) whose name contains `needle`, as flat rows with their path. */
function matches(nodes: readonly FolderNode[], needle: string, path: string[] = []): { node: FolderNode; path: string }[] {
  const out: { node: FolderNode; path: string }[] = [];
  for (const node of nodes) {
    const here = [...path, node.label];
    if (node.folder && node.label.toLocaleLowerCase().includes(needle)) out.push({ node, path: here.join(' / ') });
    out.push(...matches(node.children, needle, here));
  }
  return out;
}

/**
 * "Move to" or "Copy to": the account's folders as a tree, with a filter
 * box. Messages from several accounts can only go to a folder every account
 * has (Inbox, Archive, Trash), each to its own account's.
 */
export function FolderPicker({ mode, rows, accounts, folders, onPick, onClose }: Props) {
  const id = useId();
  const [filter, setFilter] = useState('');
  const accountIds = [...new Set(rows.map((r) => r.accountId))];
  const single = accountIds.length === 1 ? accountIds[0] : undefined;
  const account = single === undefined ? undefined : accounts.find((a) => a.id === single);
  const own = single === undefined ? [] : (folders[single] ?? []);
  const here = new Set(rows.map((r) => r.folderId));
  /** Every message is already in it: moving there would do nothing. */
  const isHere = (folder: Folder) => mode === 'move' && here.size === 1 && here.has(folder.id);
  const verb = mode === 'move' ? 'Move' : 'Copy';
  const count = rows.length === 1 ? 'this message' : `${String(rows.length)} messages`;
  const needle = filter.trim().toLocaleLowerCase();
  const tree = buildFolderTree(own);

  const pick = (folder: Folder, label: string) => {
    onPick({ folderId: folder.id }, label);
  };

  const folderButton = (folder: Folder, label: string, depth: number, title?: string) => (
    <button
      type="button"
      className="picker__folder"
      style={{ '--depth': depth } as CSSProperties}
      disabled={isHere(folder)}
      aria-label={title ?? label}
      onClick={() => {
        pick(folder, label);
      }}
    >
      <Icon name={ICONS[folder.role] ?? 'folder'} />
      <span className="picker__name">{label}</span>
      {isHere(folder) && <span className="muted small">(here now)</span>}
    </button>
  );

  const renderTree = (nodes: readonly FolderNode[], depth: number) => (
    <ul className="picker__list">
      {nodes.map((node) => (
        <li key={node.key}>
          {node.folder ? (
            folderButton(node.folder, node.label, depth)
          ) : (
            <span className="picker__folder picker__folder--container" style={{ '--depth': depth } as CSSProperties}>
              <Icon name="folder" />
              <span className="picker__name">{node.label}</span>
            </span>
          )}
          {node.children.length > 0 && renderTree(node.children, depth + 1)}
        </li>
      ))}
    </ul>
  );

  return (
    <Modal
      title={`${verb} ${count} to…`}
      description={
        account
          ? `Folders in ${account.displayName || account.email}.`
          : 'These messages are in different accounts, so they can only go to a folder every account has. Each goes to its own account’s.'
      }
      onClose={onClose}
    >
      {account ? (
        <>
          <label className="form-field" htmlFor={`${id}-filter`}>
            <span className="form-field__label">Find a folder</span>
            <input
              id={`${id}-filter`}
              className="field"
              type="search"
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value);
              }}
            />
          </label>
          <nav className="picker" aria-label="Folders">
            {needle ? (
              (() => {
                const found = matches(tree, needle);
                return found.length === 0 ? (
                  <p className="muted">No folder matches “{filter.trim()}”.</p>
                ) : (
                  <ul className="picker__list">
                    {found.map(({ node, path }) =>
                      node.folder ? <li key={node.key}>{folderButton(node.folder, path, 0)}</li> : null,
                    )}
                  </ul>
                );
              })()
            ) : (
              renderTree(tree, 0)
            )}
          </nav>
        </>
      ) : (
        <nav className="picker" aria-label="Folders">
          <ul className="picker__list">
            {SHARED_ROLES.map((role) => (
              <li key={role}>
                <button
                  type="button"
                  className="picker__folder"
                  onClick={() => {
                    onPick({ role }, ROLE_TITLES[role]);
                  }}
                >
                  <Icon name={ICONS[role] ?? 'folder'} />
                  <span className="picker__name">{ROLE_TITLES[role]}</span>
                </button>
              </li>
            ))}
          </ul>
        </nav>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
      </div>
    </Modal>
  );
}
