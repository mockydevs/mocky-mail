import { FOLDER_IDS, type Account, type FolderCounts, type FolderId, type Label } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { FOLDER_TITLES } from './selectors';

interface Props {
  folder: FolderId;
  accountId: string | null;
  labelId: string | null;
  accounts: readonly Account[];
  labels: readonly Label[];
  counts: FolderCounts | null;
  onFolder: (folder: FolderId) => void;
  onAccount: (accountId: string) => void;
  onLabel: (labelId: string) => void;
  onCompose: () => void;
}

function FolderCount({ folder, counts }: { folder: FolderId; counts: FolderCounts | null }) {
  if (!counts) return null;
  if (folder === 'inbox' && counts.inboxUnread > 0) {
    return (
      <span className="nav__count nav__count--strong" aria-label={`${counts.inboxUnread} unread`}>
        {counts.inboxUnread}
      </span>
    );
  }
  if (folder === 'outbox' && counts.outbox > 0) {
    return (
      <span className="nav__badge" aria-label={`${counts.outbox} waiting`}>
        {counts.outbox}
      </span>
    );
  }
  if (folder === 'drafts' && counts.drafts > 0) {
    return <span className="nav__count">{counts.drafts}</span>;
  }
  return null;
}

export function Sidebar(props: Props) {
  const { folder, accountId, labelId, accounts, labels, counts } = props;
  return (
    <nav className="sidebar" aria-label="Mailboxes">
      <button type="button" className="btn btn--primary btn--large btn--block" onClick={props.onCompose}>
        <Icon name="pen" />
        New message
      </button>

      <ul className="nav" aria-label="Folders">
        {FOLDER_IDS.map((id) => (
          <li key={id}>
            <button
              type="button"
              className="nav__item"
              aria-current={id === folder ? 'page' : undefined}
              onClick={() => {
                props.onFolder(id);
              }}
            >
              <Icon name={id} />
              {FOLDER_TITLES[id]}
              <FolderCount folder={id} counts={counts} />
            </button>
          </li>
        ))}
      </ul>

      <div className="nav-group">
        <h2 className="nav-group__title" id="accounts-title">
          Accounts
        </h2>
        <ul className="nav" aria-labelledby="accounts-title">
          {accounts.map((account) => {
            const unread = counts?.unreadByAccount[account.id] ?? 0;
            return (
              <li key={account.id}>
                <button
                  type="button"
                  className="nav__item nav__item--account"
                  aria-pressed={accountId === account.id}
                  onClick={() => {
                    props.onAccount(account.id);
                  }}
                >
                  <span className={`dot dot--${account.hue}`} aria-hidden="true" />
                  <span className="nav__stack">
                    <span>{account.label}</span>
                    <span className="nav__sub">{account.address}</span>
                  </span>
                  {unread > 0 && (
                    <span className="nav__count nav__count--strong" aria-label={`${unread} unread`}>
                      {unread}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      </div>

      <div className="nav-group">
        <h2 className="nav-group__title" id="labels-title">
          Labels
        </h2>
        <ul className="nav" aria-labelledby="labels-title">
          {labels.map((label) => (
            <li key={label.id}>
              <button
                type="button"
                className={`nav__item hue--${label.hue}`}
                aria-pressed={labelId === label.id}
                onClick={() => {
                  props.onLabel(label.id);
                }}
              >
                <Icon name="tag" />
                {label.name}
              </button>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  );
}
