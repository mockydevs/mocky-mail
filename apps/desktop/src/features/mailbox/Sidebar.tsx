import { useState, type CSSProperties } from 'react';

import type { Account, Folder, FolderRole } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { buildFolderTree, type FolderNode } from './folderTree';
import { sameView, unifiedCounts, type MailView } from './selectors';
import { useCollapsedFolders } from './useCollapsedFolders';

interface Props {
  view: MailView;
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  outboxCount: number;
  onView: (view: MailView) => void;
  onCompose: () => void;
  onSettings: () => void;
  onAddAccount: () => void;
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

function NavButton({
  active,
  icon,
  label,
  count,
  suffix,
  badge,
  onClick,
}: {
  active: boolean;
  icon: IconName;
  label: string;
  count?: number;
  /** Words read by screen readers after the count, such as "unread". */
  suffix?: string;
  badge?: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="nav__item" aria-current={active ? 'page' : undefined} onClick={onClick}>
      <Icon name={icon} />
      <span className="nav__label">{label}</span>
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

interface FolderItemsProps {
  nodes: FolderNode[];
  depth: number;
  accountId: string;
  view: MailView;
  collapsed: ReturnType<typeof useCollapsedFolders>;
  onView: (view: MailView) => void;
}

/** One level of an account's folder tree; parents can be collapsed. */
function FolderItems({ nodes, depth, accountId, view, collapsed, onView }: FolderItemsProps) {
  return (
    <>
      {nodes.map((node) => {
        const hasChildren = node.children.length > 0;
        const expanded = hasChildren && !collapsed.isCollapsed(node.key);
        const folder = node.folder;
        const target: MailView | null = folder ? { type: 'folder', accountId, folderId: folder.id } : null;
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
                  count={folder.role === 'drafts' || folder.role === 'sent' || folder.role === 'trash' ? 0 : folder.unread}
                  suffix="unread"
                  onClick={() => {
                    onView(target);
                  }}
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

export function Sidebar({ view, accounts, folders, outboxCount, onView, onCompose, onSettings, onAddAccount }: Props) {
  const collapsed = useCollapsedFolders();
  const [openAccountId, setOpenAccountId] = useOpenAccount(view, accounts);
  const all = Object.values(folders).flat();
  const counts = unifiedCounts(all);
  const unified = (role: 'inbox' | 'drafts' | 'sent'): MailView => ({ type: 'unified', role });

  return (
    <nav className="sidebar" aria-label="Mailboxes">
      <button type="button" className="btn btn--primary btn--large btn--block" onClick={onCompose}>
        <Icon name="pen" />
        New message
      </button>

      <div className="sidebar__scroll">
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
          return (
            <div className="nav-group nav-group--account" key={account.id}>
              <h2 className="nav-group__heading">
                <button
                  type="button"
                  className="account-toggle"
                  aria-expanded={expanded}
                  aria-controls={expanded ? listId : undefined}
                  aria-label={`${name} (${account.email})${unread > 0 ? `, ${String(unread)} unread` : ''}`}
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
                    view={view}
                    collapsed={collapsed}
                    onView={onView}
                  />
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
          <button type="button" className="nav__item" onClick={onSettings}>
            <Icon name="settings" />
            Settings
          </button>
        </div>
      </div>
    </nav>
  );
}
