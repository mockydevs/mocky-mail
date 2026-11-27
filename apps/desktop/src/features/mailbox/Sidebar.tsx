import { useState, type CSSProperties, type DragEvent, type MouseEvent } from 'react';

import type { Account, Folder, FolderRole } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { ContextMenu, MenuItem, MenuSeparator } from '../../ui/Menu';
import { buildFolderTree, type FolderNode } from './folderTree';
import { folderLabel, sameView, unifiedCounts, type MailView } from './selectors';
import { useCollapsedFolders } from './useCollapsedFolders';

/** Messages being dragged from the list: the one account they all belong to, or null when they are from several. */
export interface MessageDrag {
  accountId: string | null;
}

interface Props {
  view: MailView;
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  outboxCount: number;
  onView: (view: MailView) => void;
  onCompose: () => void;
  onSettings: () => void;
  onContacts: () => void;
  onCalendar: () => void;
  /** Open Tasks; missing: no Tasks entry. */
  onTasks?: () => void;
  /** Open "Manage folders" for an account. */
  onManageFolders: (accountId: string) => void;
  onAddAccount: () => void;
  /** Messages being dragged over the sidebar, if any. */
  drag?: MessageDrag | null;
  /** Dropped on a folder: move them there, or copy with Ctrl held. */
  onDropFolder?: (folder: Folder, copy: boolean) => void;
  /** Add a folder to Favourites, or take it off. */
  onFavorite?: (folder: Folder, favorite: boolean) => void;
  /** Every account id in the new order, after dragging one. */
  onReorderAccounts?: (ids: string[]) => void;
  /** "Import messages into this folder…" in a folder's menu. */
  onImportInto?: (folder: Folder) => void;
  /** "Export folder as .mbox…" in a folder's menu. */
  onExportFolder?: (folder: Folder) => void;
}

const ROLE_ICONS: Readonly<Record<FolderRole, IconName>> = {
  inbox: 'inbox',
  drafts: 'drafts',
  sent: 'sent',
  archive: 'archive',
  junk: 'junk',
  trash: 'trash',
  all: 'all',
  flagged: 'flag',
  other: 'folder',
};

/** Drop handling for a folder button, while messages are dragged. */
interface DropProps {
  /** These messages may go here. */
  accepts: boolean;
  over: boolean;
  onEnter: () => void;
  onLeave: () => void;
  onDrop: (copy: boolean) => void;
}

function wantsCopy(event: DragEvent<HTMLElement>): boolean {
  // Browsers report Ctrl (Option on a Mac) as a copy; the key itself counts too.
  const transfer = event.dataTransfer as DataTransfer | null | undefined;
  return event.ctrlKey || event.metaKey || transfer?.dropEffect === 'copy';
}

function NavButton({
  active,
  icon,
  label,
  count,
  suffix,
  badge,
  onClick,
  onContextMenu,
  drop,
  dot,
}: {
  active: boolean;
  icon: IconName;
  label: string;
  count?: number;
  /** Words read by screen readers after the count, such as "unread". */
  suffix?: string;
  badge?: boolean;
  onClick: () => void;
  onContextMenu?: (event: MouseEvent<HTMLButtonElement>) => void;
  drop?: DropProps;
  /** An account colour beside the label (Favourites from several accounts). */
  dot?: { color: string; name: string };
}) {
  const dropClass = drop?.accepts ? (drop.over ? ' nav__item--drop nav__item--drop-over' : ' nav__item--drop') : '';
  return (
    <button
      type="button"
      className={`nav__item${dropClass}`}
      aria-current={active ? 'page' : undefined}
      onClick={onClick}
      onContextMenu={onContextMenu}
      onDragEnter={(e) => {
        if (!drop?.accepts) return;
        e.preventDefault();
        drop.onEnter();
      }}
      onDragOver={(e) => {
        if (!drop?.accepts) return;
        e.preventDefault();
        const transfer = e.dataTransfer as DataTransfer | null | undefined;
        if (transfer) transfer.dropEffect = e.ctrlKey || e.metaKey ? 'copy' : 'move';
      }}
      onDragLeave={() => {
        if (drop?.accepts) drop.onLeave();
      }}
      onDrop={(e) => {
        if (!drop?.accepts) return;
        e.preventDefault();
        drop.onDrop(wantsCopy(e));
      }}
    >
      <Icon name={icon} />
      <span className="nav__label">{label}</span>
      {dot && (
        <>
          <span className="dot dot--small" style={{ background: dot.color }} title={dot.name} aria-hidden="true" />
          {' '}
          <span className="visually-hidden">({dot.name})</span>
        </>
      )}
      {count !== undefined && count > 0 && ' '}
      {count !== undefined && count > 0 && (
        <span className={badge ? 'nav__badge' : 'nav__count nav__count--strong'}>
          {count}
          {suffix && ' '}
          {suffix && <span className="visually-hidden">{suffix}</span>}
        </span>
      )}
    </button>
  );
}

/** Unread count shown beside a folder; Drafts, Sent and Trash show none. */
function folderCount(folder: Folder): number {
  return folder.role === 'drafts' || folder.role === 'sent' || folder.role === 'trash' ? 0 : folder.unread;
}

interface FolderTools {
  view: MailView;
  onView: (view: MailView) => void;
  /** Right-click or long press on a folder. */
  onMenu: (folder: Folder, event: MouseEvent<HTMLButtonElement>) => void;
  dropFor: (folder: Folder) => DropProps | undefined;
}

interface FolderItemsProps extends FolderTools {
  nodes: FolderNode[];
  depth: number;
  accountId: string;
  collapsed: ReturnType<typeof useCollapsedFolders>;
}

/** One level of an account's folder tree; parents can be collapsed. */
function FolderItems({ nodes, depth, accountId, view, collapsed, onView, onMenu, dropFor }: FolderItemsProps) {
  return (
    <>
      {nodes.map((node) => {
        const hasChildren = node.children.length > 0;
        const expanded = hasChildren && !collapsed.isCollapsed(node.key);
        const folder = node.folder;
        const target: MailView | null = folder ? { type: 'folder', accountId, folderId: folder.id } : null;
        const drop = folder ? dropFor(folder) : undefined;
        return (
          <li key={node.key}>
            <div
              className={hasChildren ? 'tree-row tree-row--parent' : 'tree-row'}
              style={{ '--depth': depth } as CSSProperties}
            >
              {folder && target ? (
                <NavButton
                  active={sameView(view, target)}
                  icon={ROLE_ICONS[folder.role]}
                  label={node.label}
                  count={folderCount(folder)}
                  suffix="unread"
                  onClick={() => {
                    onView(target);
                  }}
                  onContextMenu={(e) => {
                    onMenu(folder, e);
                  }}
                  {...(drop ? { drop } : {})}
                />
              ) : (
                <span className="nav__item nav__item--container">
                  <Icon name="folder" />
                  <span className="nav__label">{node.label}</span>
                </span>
              )}
              {hasChildren && (
                <button
                  type="button"
                  className="tree-toggle"
                  aria-expanded={expanded}
                  aria-label={`${expanded ? 'Collapse' : 'Expand'} ${node.label}`}
                  onClick={() => {
                    collapsed.toggle(node.key);
                  }}
                >
                  <span className={expanded ? 'tree-chevron' : 'tree-chevron tree-chevron--closed'}>
                    <Icon name="chevron" size={14} />
                  </span>
                </button>
              )}
            </div>
            {expanded && (
              <ul className="nav nav--nested" aria-label={`${node.label} folders`}>
                <FolderItems
                  nodes={node.children}
                  depth={depth + 1}
                  accountId={accountId}
                  view={view}
                  collapsed={collapsed}
                  onView={onView}
                  onMenu={onMenu}
                  dropFor={dropFor}
                />
              </ul>
            )}
          </li>
        );
      })}
    </>
  );
}

/**
 * Which account's folders are shown. Only one account is open at a time, like
 * Outlook, so a second account is one click away instead of a long scroll.
 * Opening a folder in another account (from search or a shortcut, say) opens
 * that account; otherwise the person's last choice stays, including none.
 */
function useOpenAccount(view: MailView, accounts: readonly Account[]) {
  const viewAccount = view.type === 'folder' ? view.accountId : null;
  const [openId, setOpenId] = useState<string | null>(viewAccount ?? accounts[0]?.id ?? null);
  const [seenViewAccount, setSeenViewAccount] = useState(viewAccount);
  if (viewAccount !== seenViewAccount) {
    setSeenViewAccount(viewAccount);
    if (viewAccount) setOpenId(viewAccount);
  }
  // Accounts load after the first render; open the first one once they arrive.
  const [seenFirst, setSeenFirst] = useState(accounts[0]?.id ?? null);
  const first = accounts[0]?.id ?? null;
  if (first !== seenFirst) {
    setSeenFirst(first);
    if (openId === null || !accounts.some((a) => a.id === openId)) setOpenId(viewAccount ?? first);
  }
  return [openId, setOpenId] as const;
}

/** Account ids with `moved` taken out and put where `target` is. */
export function reorder(ids: readonly string[], moved: string, target: string): string[] {
  const from = ids.indexOf(moved);
  const to = ids.indexOf(target);
  if (from < 0 || to < 0 || from === to) return [...ids];
  const next = ids.filter((id) => id !== moved);
  next.splice(to, 0, moved);
  return next;
}

export function Sidebar({
  view,
  accounts,
  folders,
  outboxCount,
  onView,
  onCompose,
  onSettings,
  onContacts,
  onCalendar,
  onTasks,
  onManageFolders,
  onAddAccount,
  drag = null,
  onDropFolder,
  onFavorite,
  onReorderAccounts,
  onImportInto,
  onExportFolder,
}: Props) {
  const collapsed = useCollapsedFolders();
  const [openAccountId, setOpenAccountId] = useOpenAccount(view, accounts);
  const all = Object.values(folders).flat();
  const counts = unifiedCounts(all);
  const unified = (role: 'inbox' | 'drafts' | 'sent'): MailView => ({ type: 'unified', role });
  /** The folder a dragged message is over. */
  const [over, setOver] = useState<string | null>(null);
  /** The account being dragged to a new place. */
  const [movingAccount, setMovingAccount] = useState<string | null>(null);
  const [accountOver, setAccountOver] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ folder: Folder; x: number; y: number } | null>(null);

  const dropFor = (folder: Folder): DropProps | undefined => {
    if (!drag || !onDropFolder) return undefined;
    return {
      accepts: drag.accountId === folder.accountId,
      over: over === folder.id,
      onEnter: () => {
        setOver(folder.id);
      },
      onLeave: () => {
        setOver((current) => (current === folder.id ? null : current));
      },
      onDrop: (copy) => {
        setOver(null);
        onDropFolder(folder, copy);
      },
    };
  };

  const openMenu = (folder: Folder, event: MouseEvent<HTMLButtonElement>) => {
    if (!onFavorite) return;
    event.preventDefault();
    const box = event.currentTarget.getBoundingClientRect();
    // A long press reports where the finger was; a keyboard's menu key reports 0, 0.
    const x = event.clientX || box.left + 24;
    const y = event.clientY || box.bottom;
    setMenu({ folder, x, y });
  };

  const tools: FolderTools = { view, onView, onMenu: openMenu, dropFor };
  const favourites = all.filter((f) => f.favorite);
  const accountName = (id: string) => {
    const account = accounts.find((a) => a.id === id);
    return account ? account.displayName || account.email : '';
  };

  return (
    <nav className="sidebar" aria-label="Mailboxes">
      <button type="button" className="btn btn--primary btn--large btn--block" onClick={onCompose}>
        <Icon name="pen" />
        New message
      </button>

      <div className="sidebar__scroll">
        {favourites.length > 0 && (
          <div className="nav-group">
            <h2 className="nav-group__title" id="favourites-title">
              Favourites
            </h2>
            <ul className="nav" aria-labelledby="favourites-title">
              {favourites.map((folder) => {
                const target: MailView = { type: 'folder', accountId: folder.accountId, folderId: folder.id };
                const account = accounts.find((a) => a.id === folder.accountId);
                const drop = dropFor(folder);
                return (
                  <li key={folder.id}>
                    <NavButton
                      active={sameView(view, target)}
                      icon={folder.role === 'other' ? 'star' : ROLE_ICONS[folder.role]}
                      label={folderLabel(folder)}
                      count={folderCount(folder)}
                      suffix="unread"
                      onClick={() => {
                        onView(target);
                      }}
                      onContextMenu={(e) => {
                        openMenu(folder, e);
                      }}
                      {...(drop ? { drop } : {})}
                      {...(accounts.length > 1 && account
                        ? { dot: { color: account.color, name: accountName(account.id) } }
                        : {})}
                    />
                  </li>
                );
              })}
            </ul>
          </div>
        )}

        <div className="nav-group">
          <h2 className="nav-group__title" id="unified-title">
            All accounts
          </h2>
          <ul className="nav" aria-labelledby="unified-title">
            <li>
              <NavButton
                active={sameView(view, unified('inbox'))}
                icon="inbox"
                label="Inbox"
                count={counts.inboxUnread}
                suffix="unread"
                onClick={() => {
                  onView(unified('inbox'));
                }}
              />
            </li>
            <li>
              <NavButton
                active={view.type === 'outbox'}
                icon="outbox"
                label="Outbox"
                count={outboxCount}
                suffix="waiting"
                badge
                onClick={() => {
                  onView({ type: 'outbox' });
                }}
              />
            </li>
            <li>
              <NavButton
                active={sameView(view, unified('drafts'))}
                icon="drafts"
                label="Drafts"
                count={counts.drafts}
                onClick={() => {
                  onView(unified('drafts'));
                }}
              />
            </li>
            <li>
              <NavButton
                active={sameView(view, unified('sent'))}
                icon="sent"
                label="Sent"
                onClick={() => {
                  onView(unified('sent'));
                }}
              />
            </li>
          </ul>
        </div>

        <div className="account-groups">
        {accounts.map((account) => {
          const nameId = `acct-${account.id}`;
          const listId = `acct-${account.id}-folders`;
          const accountFolders = folders[account.id] ?? [];
          const inbox = accountFolders.find((f) => f.role === 'inbox');
          const expanded = openAccountId === account.id;
          const name = account.displayName || account.email;
          const unread = !expanded && inbox ? inbox.unread : 0;
          const reorderable = onReorderAccounts !== undefined && accounts.length > 1;
          const dropHere = movingAccount !== null && movingAccount !== account.id;
          return (
            <div
              className={`nav-group nav-group--account${dropHere && accountOver === account.id ? ' nav-group--drop-over' : ''}`}
              key={account.id}
            >
              <h2 className="nav-group__heading">
                <button
                  type="button"
                  className="account-toggle"
                  aria-expanded={expanded}
                  aria-controls={expanded ? listId : undefined}
                  aria-label={`${name} (${account.email})${unread > 0 ? `, ${String(unread)} unread` : ''}`}
                  title={reorderable ? 'Drag to change the order of accounts' : undefined}
                  draggable={reorderable}
                  onDragStart={(e) => {
                    setMovingAccount(account.id);
                    const transfer = e.dataTransfer as DataTransfer | null | undefined;
                    if (transfer) {
                      transfer.effectAllowed = 'move';
                      transfer.setData('text/plain', name);
                    }
                  }}
                  onDragEnd={() => {
                    setMovingAccount(null);
                    setAccountOver(null);
                  }}
                  onDragEnter={(e) => {
                    if (!dropHere) return;
                    e.preventDefault();
                    setAccountOver(account.id);
                  }}
                  onDragOver={(e) => {
                    if (dropHere) e.preventDefault();
                  }}
                  onDragLeave={() => {
                    setAccountOver((current) => (current === account.id ? null : current));
                  }}
                  onDrop={(e) => {
                    if (!dropHere) return;
                    e.preventDefault();
                    const moved = movingAccount;
                    setMovingAccount(null);
                    setAccountOver(null);
                    onReorderAccounts?.(reorder(accounts.map((a) => a.id), moved, account.id));
                  }}
                  onClick={() => {
                    if (expanded) {
                      setOpenAccountId(null);
                      return;
                    }
                    setOpenAccountId(account.id);
                    if (inbox) onView({ type: 'folder', accountId: account.id, folderId: inbox.id });
                  }}
                >
                  <span className="dot" style={{ background: account.color }} aria-hidden="true" />
                  <span className="account-toggle__text">
                    <span className="nav-group__name" id={nameId}>
                      {name}
                    </span>
                    <span className="nav-group__sub">{account.email}</span>
                  </span>
                  {unread > 0 && (
                    <span className="nav__count nav__count--strong" aria-hidden="true">
                      {unread}
                    </span>
                  )}
                  <span className={expanded ? 'tree-chevron' : 'tree-chevron tree-chevron--closed'} aria-hidden="true">
                    <Icon name="chevron" size={14} />
                  </span>
                </button>
              </h2>
              {expanded && (
                <ul className="nav" id={listId} aria-labelledby={nameId}>
                  <FolderItems
                    nodes={buildFolderTree(accountFolders)}
                    depth={0}
                    accountId={account.id}
                    collapsed={collapsed}
                    {...tools}
                  />
                  <li>
                    <button
                      type="button"
                      className="nav__item nav__item--quiet"
                      onClick={() => {
                        onManageFolders(account.id);
                      }}
                    >
                      <Icon name="folder" />
                      Manage folders
                    </button>
                  </li>
                </ul>
              )}
            </div>
          );
        })}
        </div>

        <div className="sidebar__foot">
          <button type="button" className="nav__item nav__item--dashed" onClick={onAddAccount}>
            <Icon name="plus" />
            Add account
          </button>
          <button type="button" className="nav__item" onClick={onCalendar}>
            <Icon name="calendar" />
            Calendar
          </button>
          {onTasks && (
            <button type="button" className="nav__item" onClick={onTasks}>
              <Icon name="check" />
              Tasks
            </button>
          )}
          <button type="button" className="nav__item" onClick={onContacts}>
            <Icon name="people" />
            Contacts
          </button>
          <button type="button" className="nav__item" onClick={onSettings}>
            <Icon name="settings" />
            Settings
          </button>
        </div>
      </div>

      {menu && onFavorite && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={`${folderLabel(menu.folder)} folder`}
          onClose={() => {
            setMenu(null);
          }}
        >
          <MenuItem
            icon="star"
            onSelect={() => {
              onFavorite(menu.folder, !menu.folder.favorite);
            }}
          >
            {menu.folder.favorite ? 'Remove from Favourites' : 'Add to Favourites'}
          </MenuItem>
          <MenuItem
            icon="folder"
            onSelect={() => {
              onManageFolders(menu.folder.accountId);
            }}
          >
            Manage folders
          </MenuItem>
          {(onImportInto ?? onExportFolder) && <MenuSeparator />}
          {onImportInto && (
            <MenuItem
              icon="upload"
              onSelect={() => {
                onImportInto(menu.folder);
              }}
            >
              Import messages into this folder…
            </MenuItem>
          )}
          {onExportFolder && (
            <MenuItem
              icon="download"
              onSelect={() => {
                onExportFolder(menu.folder);
              }}
            >
              Export folder as .mbox…
            </MenuItem>
          )}
        </ContextMenu>
      )}
    </nav>
  );
}
