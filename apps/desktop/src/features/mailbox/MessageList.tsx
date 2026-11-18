import { useEffect, useRef } from 'react';

import type { Account, AppError, Focus, FolderRole, MessageListItem, ReadFilter } from '../../api/types';
import { dayGroup, formatListTime } from '../../lib/format';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { emptyCopy } from './selectors';
import { usePullToRefresh } from './usePullToRefresh';

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
  /** Focused Inbox: the half shown, or null when the inbox isn't split. */
  focus?: Focus | null;
  onFocus?: (focus: Focus) => void;
  onSelect: (id: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
  /** Phones: pull the list down to check for new mail. */
  onRefresh?: (() => Promise<void>) | undefined;
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
  const pull = usePullToRefresh(listRef, props.onRefresh);

  useEffect(() => {
    if (!selectedId) return;
    const rows = listRef.current?.querySelectorAll<HTMLElement>('[data-id]') ?? [];
    const row = [...rows].find((el) => el.dataset['id'] === selectedId);
    // scrollIntoView is missing in some test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    row?.scrollIntoView?.({ block: 'nearest' });
  }, [selectedId]);

  // Infinite scroll: watch a marker at the end of the list, relative to the
  // list's own scroll container (the window never scrolls).
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadMoreRef = useRef(props.onLoadMore);
  useEffect(() => {
    loadMoreRef.current = props.onLoadMore;
  });
  const canLoadMore = props.status === 'ready' && props.hasMore && !props.loadingMore;
  useEffect(() => {
    const root = listRef.current;
    const sentinel = sentinelRef.current;
    if (!canLoadMore || !root || !sentinel || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) loadMoreRef.current();
      },
      { root, rootMargin: `0px 0px ${String(PREFETCH_PX)}px 0px` },
    );
    observer.observe(sentinel);
    return () => {
      observer.disconnect();
    };
  }, [canLoadMore, items.length]);

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
        {props.focus && (
          <div className="focus-tabs" role="tablist" aria-label="Focused Inbox">
            {(['focused', 'other'] as const).map((f) => (
              <button
                key={f}
                type="button"
                role="tab"
                className="focus-tab"
                aria-selected={props.focus === f}
                onClick={() => {
                  props.onFocus?.(f);
                }}
              >
                {f === 'focused' ? 'Focused' : 'Other'}
              </button>
            ))}
          </div>
        )}
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

      <div className="list__scroll" ref={listRef} data-testid="list-scroll">
        {props.onRefresh && (
          <div
            className={`pull pull--${pull.state}`}
            style={{ height: pull.distance }}
            role="status"
            aria-live="polite"
          >
            {pull.state !== 'idle' && (
              <>
                {pull.state === 'refreshing' ? (
                  <span className="spinner" aria-hidden="true" />
                ) : (
                  <span className="pull__arrow" aria-hidden="true">
                    <Icon name="back" size={18} />
                  </span>
                )}
                <span>
                  {pull.state === 'refreshing'
                    ? 'Checking for new mail'
                    : pull.state === 'armed'
                      ? 'Release to check for new mail'
                      : 'Pull to check for new mail'}
                </span>
              </>
            )}
          </div>
        )}
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
            {items.map((m, i) => {
              const account = accounts.find((a) => a.id === m.accountId);
              const group = dayGroup(m.date, now);
              const previous = items[i - 1];
              const startsGroup = !previous || dayGroup(previous.date, now) !== group;
              return (
                <li key={m.id}>
                  {startsGroup && <h2 className="rows__group">{group}</h2>}
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
                        {(m.threadCount ?? 1) > 1 && (
                          <span className="row__count" aria-label={`${String(m.threadCount)} messages in this conversation`}>
                            {m.threadCount}
                          </span>
                        )}
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
          <div className="list__more" ref={sentinelRef}>
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
