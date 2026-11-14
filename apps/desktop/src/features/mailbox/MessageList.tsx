import { useEffect, useRef, type UIEvent } from 'react';

import type { Account, AppError, FolderRole, MessageListItem, ReadFilter } from '../../api/types';
import { formatListTime } from '../../lib/format';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { emptyCopy } from './selectors';

interface Props {
  title: string;
  subtitle: string;
  role: FolderRole | null;
  status: 'loading' | 'ready' | 'error';
  items: readonly MessageListItem[];
  total: number;
  error: AppError | null;
  filtered: boolean;
  filter: ReadFilter;
  selectedId: string | null;
  accounts: readonly Account[];
  now: number;
  hasMore: boolean;
  loadingMore: boolean;
  onFilter: (filter: ReadFilter) => void;
  onSelect: (id: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}

const FILTERS: readonly { id: ReadFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'unread', label: 'Unread' },
  { id: 'flagged', label: 'Flagged' },
];

/** Start loading the next page this many pixels before the end. */
const PREFETCH_PX = 400;

function who(item: MessageListItem, outgoing: boolean): string {
  if (outgoing) {
    const first = item.to[0];
    const names = item.to.map((t) => t.name ?? t.address);
    return first ? `To ${names.slice(0, 2).join(', ')}${item.to.length > 2 ? ` +${String(item.to.length - 2)}` : ''}` : 'No recipients';
  }
  return item.from ? (item.from.name ?? item.from.address) : 'Unknown sender';
}

export function MessageList(props: Props) {
  const { items, selectedId, accounts, now, role } = props;
  const outgoing = role === 'sent' || role === 'drafts';
  const listRef = useRef<HTMLDivElement>(null);
  const showAccount = accounts.length > 1;
  const empty = emptyCopy(role, props.filtered);

  useEffect(() => {
    if (!selectedId) return;
    const rows = listRef.current?.querySelectorAll<HTMLElement>('[data-id]') ?? [];
    const row = [...rows].find((el) => el.dataset['id'] === selectedId);
    // scrollIntoView is missing in some test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    row?.scrollIntoView?.({ block: 'nearest' });
  }, [selectedId]);

  const onScroll = (event: UIEvent<HTMLDivElement>) => {
    const el = event.currentTarget;
    if (props.hasMore && !props.loadingMore && el.scrollTop + el.clientHeight >= el.scrollHeight - PREFETCH_PX) {
      props.onLoadMore();
    }
  };

  return (
    <section className="list" aria-labelledby="list-title">
      <div className="list__head">
        <div className="list__heading">
          <h1 id="list-title" className="list__title">
            {props.title}
          </h1>
          <span className="muted small">
            {props.status === 'ready' ? `${String(props.total)} ${props.total === 1 ? 'message' : 'messages'}, ` : ''}
            {props.subtitle}
          </span>
        </div>
        <div className="tabs" role="group" aria-label="Show">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              className="tab"
              aria-pressed={props.filter === f.id}
              onClick={() => {
                props.onFilter(f.id);
              }}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      <div className="list__scroll" ref={listRef} onScroll={onScroll}>
        {props.status === 'loading' && (
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

        {props.status === 'error' && (
          <div className="empty" role="alert">
            <p className="empty__title">Could not load {props.title}</p>
            <p className="empty__text">{props.error?.message}</p>
            <button type="button" className="btn btn--secondary" onClick={props.onRetry}>
              Try again
            </button>
          </div>
        )}

        {props.status === 'ready' && items.length === 0 && (
          <div className="empty">
            <Icon name="inbox" size={40} />
            <p className="empty__title">{empty.title}</p>
            <p className="empty__text">{empty.text}</p>
          </div>
        )}

        {props.status === 'ready' && items.length > 0 && (
          <ul className="rows" aria-label={`${props.title} messages`}>
            {items.map((m) => {
              const account = accounts.find((a) => a.id === m.accountId);
              return (
                <li key={m.id}>
                  <button
                    type="button"
                    data-id={m.id}
                    className={`row${m.unread ? ' row--unread' : ''}`}
                    aria-current={m.id === selectedId ? 'true' : undefined}
                    onClick={() => {
                      props.onSelect(m.id);
                    }}
                  >
                    <Avatar
                      name={outgoing ? (m.to[0]?.name ?? m.to[0]?.address ?? '?') : (m.from?.name ?? m.from?.address ?? '?')}
                      address={outgoing ? (m.to[0]?.address ?? '') : (m.from?.address ?? '')}
                    />
                    <span className="row__text">
                      <span className="row__top">
                        {m.unread && <span className="row__unread" aria-label="Unread" />}
                        <span className="row__who">{who(m, outgoing)}</span>
                        {m.answered && (
                          <span className="row__icon" aria-label="Replied">
                            <Icon name="answered" size={14} />
                          </span>
                        )}
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
                          {showAccount && account && (
                            <span
                              className="dot dot--small"
                              style={{ background: account.color }}
                              aria-label={account.email}
                              role="img"
                            />
                          )}
                          {formatListTime(m.date, now)}
                        </span>
                      </span>
                      <span className="row__subject">{m.subject || '(no subject)'}</span>
                      <span className="row__preview">{m.preview}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}

        {props.status === 'ready' && props.hasMore && (
          <div className="list__more">
            <button type="button" className="btn btn--secondary" disabled={props.loadingMore} onClick={props.onLoadMore}>
              {props.loadingMore ? 'Loading' : `Load more (${String(props.total - items.length)} left)`}
            </button>
          </div>
        )}
        {props.error && props.status === 'ready' && (
          <p className="form-error" role="alert">
            {props.error.message}
          </p>
        )}
      </div>
    </section>
  );
}
