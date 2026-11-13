import type { Account, FolderId, MessageSummary } from '../../api/types';
import { formatListTime } from '../../lib/format';
import type { Loaded } from '../../lib/useLoad';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { outboxCopy } from '../outbox/outboxCopy';
import { emptyCopy, FOLDER_TITLES, listMeta, type ReadFilter } from './selectors';

interface Props {
  folder: FolderId;
  loaded: Loaded<readonly MessageSummary[]>;
  visible: readonly MessageSummary[];
  filtered: boolean;
  selectedId: string | null;
  read: ReadFilter;
  accounts: readonly Account[];
  accountLabel: string | null;
  now: Date;
  onSelect: (id: string) => void;
  onRead: (filter: ReadFilter) => void;
  onRetry: () => void;
}

const READ_FILTERS: readonly { id: ReadFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'unread', label: 'Unread' },
  { id: 'flagged', label: 'Flagged' },
];

export function MessageList(props: Props) {
  const { folder, loaded, visible, selectedId, accounts, now } = props;
  const empty = emptyCopy(folder, props.filtered);
  const hueOf = (accountId: string) => accounts.find((a) => a.id === accountId)?.hue ?? 'blue';
  const labelOf = (accountId: string) => accounts.find((a) => a.id === accountId)?.label ?? accountId;

  return (
    <section className="list" aria-labelledby="list-title">
      <div className="list__head">
        <div className="list__heading">
          <h1 id="list-title" className="list__title">
            {FOLDER_TITLES[folder]}
          </h1>
          {loaded.status === 'ready' && <span className="muted small">{listMeta(visible.length, props.accountLabel)}</span>}
        </div>
        <div className="tabs" role="group" aria-label="Show">
          {READ_FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              className="tab"
              aria-pressed={props.read === f.id}
              onClick={() => {
                props.onRead(f.id);
              }}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {folder === 'outbox' && (
        <p className="list__note">
          <Icon name="clock" />
          Messages here are saved on this computer and send automatically when the connection is good.
        </p>
      )}

      <div className="list__scroll">
        {loaded.status === 'loading' && (
          <div aria-busy="true" aria-label="Loading messages">
            {[0, 1, 2, 3, 4].map((i) => (
              <div key={i} className="row row--skeleton">
                <div className="skeleton skeleton--avatar" />
                <div className="row__text">
                  <div className="skeleton skeleton--line" />
                  <div className="skeleton skeleton--line skeleton--short" />
                </div>
              </div>
            ))}
          </div>
        )}

        {loaded.status === 'error' && (
          <div className="empty" role="alert">
            <p className="empty__title">Could not load {FOLDER_TITLES[folder]}</p>
            <p className="empty__text">{loaded.message}</p>
            <button type="button" className="btn btn--secondary" onClick={props.onRetry}>
              Try again
            </button>
          </div>
        )}

        {loaded.status === 'ready' && visible.length === 0 && (
          <div className="empty">
            <Icon name="inbox" size={40} />
            <p className="empty__title">{empty.title}</p>
            <p className="empty__text">{empty.text}</p>
          </div>
        )}

        {loaded.status === 'ready' && visible.length > 0 && (
          <ul className="rows" aria-label={`${FOLDER_TITLES[folder]} messages`}>
            {visible.map((m) => (
              <li key={m.id}>
                <button
                  type="button"
                  className={`row${m.unread ? ' row--unread' : ''}`}
                  aria-current={m.id === selectedId ? 'true' : undefined}
                  onClick={() => {
                    props.onSelect(m.id);
                  }}
                >
                  <Avatar name={m.counterpart.name} address={m.counterpart.address} />
                  <span className="row__text">
                    <span className="row__top">
                      {m.unread && <span className="row__unread" aria-label="Unread" />}
                      <span className="row__who">
                        {m.direction === 'outgoing' ? 'To ' : ''}
                        {m.counterpart.name || m.counterpart.address}
                      </span>
                      {m.hasAttachments && (
                        <span className="row__icon" aria-label="Has attachments">
                          <Icon name="paperclip" size={14} />
                        </span>
                      )}
                      {m.flagged && (
                        <span className="row__icon row__icon--flag" aria-label="Flagged">
                          <Icon name="flag" size={14} filled />
                        </span>
                      )}
                      <span className="row__time">
                        <span className={`dot dot--small dot--${hueOf(m.accountId)}`} aria-hidden="true" />
                        <time dateTime={m.date}>{formatListTime(m.date, now)}</time>
                      </span>
                    </span>
                    <span className="row__subject">{m.subject || '(no subject)'}</span>
                    <span className="row__preview">{m.preview}</span>
                    {m.outbox && (
                      <span className={`pill pill--${outboxCopy(m.outbox, labelOf(m.accountId)).tone}`}>
                        {outboxCopy(m.outbox, labelOf(m.accountId)).pill}
                      </span>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
