import type { Account, Folder, FolderRole } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { folderLabel, sameView, sortFolders, unifiedCounts, type MailView } from './selectors';

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

export function Sidebar({ view, accounts, folders, outboxCount, onView, onCompose, onSettings, onAddAccount }: Props) {
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

        {accounts.map((account) => {
          const titleId = `acct-${account.id}`;
          return (
            <div className="nav-group" key={account.id}>
              <h2 className="nav-group__title nav-group__title--account" id={titleId}>
                <span className="dot" style={{ background: account.color }} aria-hidden="true" />
                <span className="nav-group__name">{account.displayName || account.email}</span>
              </h2>
              <p className="nav-group__sub">{account.email}</p>
              <ul className="nav" aria-labelledby={titleId}>
                {sortFolders(folders[account.id] ?? []).map((folder) => {
                  const target: MailView = { type: 'folder', accountId: account.id, folderId: folder.id };
                  return (
                    <li key={folder.id}>
                      <NavButton
                        active={sameView(view, target)}
                        icon={ROLE_ICONS[folder.role]}
                        label={folderLabel(folder)}
                        count={folder.role === 'drafts' || folder.role === 'sent' || folder.role === 'trash' ? 0 : folder.unread}
                        suffix="unread"
                        onClick={() => {
                          onView(target);
                        }}
                      />
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })}

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
