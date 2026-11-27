import { useEffect, useRef, useState, type CSSProperties, type DragEvent, type MouseEvent, type ReactNode } from 'react';

import type { Account, AppError, Category, Focus, FolderRole, MessageListItem, ReadFilter } from '../../api/types';
import { formatListTime } from '../../lib/format';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { MenuButton, MenuItem } from '../../ui/Menu';
import { dueLabel, isOverdue } from './flagDue';
import { groupRows, type RowGroup } from './grouping';
import { emptyCopy } from './selectors';
import { usePullToRefresh } from './usePullToRefresh';

/** How a row was picked for the selection: Ctrl/Cmd-click, Shift-click, its checkbox or a tap in selection mode. */
export type PickMode = 'toggle' | 'range' | 'check';

interface Props {
  title: string;
  subtitle: string;
  role: FolderRole | null;
  status: 'loading' | 'ready' | 'error';
  /** The rows under their headings, in the order shown. */
  groups?: readonly RowGroup[];
  /** Or just the rows, put under day headings. */
  items?: readonly MessageListItem[];
  total: number;
  error: AppError | null;
  filtered: boolean;
  filter: ReadFilter;
  selectedId: string | null;
  accounts: readonly Account[];
  /** Colours for category chips. */
  categories?: readonly Category[];
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
  /** Phone layout: no hover, long press to select, no dragging. */
  phone?: boolean;
  /** Rows ticked for a bulk action. */
  checked?: ReadonlySet<string>;
  onPick?: (id: string, mode: PickMode) => void;
  /** Phones: a long press starts selecting with this row. */
  onLongPress?: (id: string) => void;
  /** A click on the row's left edge (the unread bar). */
  onToggleRead?: (id: string) => void;
  /** Computers: double-click opens the message in a window of its own. */
  onOpenWindow?: ((id: string) => void) | undefined;
  /** Computers: a row is dragged; the shell decides what goes with it. */
  onDragRows?: ((id: string, event: DragEvent<HTMLElement>) => void) | undefined;
  onDragEnd?: () => void;
  /** Computers: actions shown over the row under the pointer. */
  rowActions?: ((item: MessageListItem) => ReactNode) | undefined;
  /** Shown in place of the header while rows are ticked. */
  selectionBar?: ReactNode;
  /** Buttons beside the title: sort and the reading pane position. */
  tools?: ReactNode;
  /** Searching beyond the open folder: the folder each row is in. */
  folderOf?: ((item: MessageListItem) => string | null) | undefined;
}

const FILTERS: readonly { id: ReadFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'unread', label: 'Unread' },
  { id: 'flagged', label: 'Flagged' },
];

/** Filters in the "More" menu next to the three tabs. */
export const MORE_FILTERS: readonly { id: ReadFilter; label: string }[] = [
  { id: 'attachments', label: 'Has attachments' },
  { id: 'toMe', label: 'To me' },
  { id: 'pinned', label: 'Pinned' },
];

/** Start loading the next page this many pixels before the end. */
const PREFETCH_PX = 400;

/** Hold this long on a phone to start selecting. */
export const LONG_PRESS_MS = 500;

function who(item: MessageListItem, outgoing: boolean): string {
  if (outgoing) {
    const first = item.to[0];
    const names = item.to.map((t) => t.name ?? t.address);
    return first ? `To ${names.slice(0, 2).join(', ')}${item.to.length > 2 ? ` +${String(item.to.length - 2)}` : ''}` : 'No recipients';
  }
  return item.from ? (item.from.name ?? item.from.address) : 'Unknown sender';
}

/** Category chip colours by name, case-insensitive. */
export function colourOf(categories: readonly Category[] | undefined, name: string): string {
  return categories?.find((c) => c.name.toLowerCase() === name.toLowerCase())?.color ?? '#6b7280';
}

export function MessageList(props: Props) {
  const { selectedId, accounts, now, role, phone = false } = props;
  const groups = props.groups ?? groupRows(props.items ?? [], 'date', now);
  const checked = props.checked ?? new Set<string>();
  const selecting = checked.size > 0;
  const items = groups.flatMap((g) => g.items);
  const outgoing = role === 'sent' || role === 'drafts';
  const listRef = useRef<HTMLDivElement>(null);
  const showAccount = accounts.length > 1;
  const empty = emptyCopy(role, props.filtered);
  const pull = usePullToRefresh(listRef, props.onRefresh);
  /** The row under the pointer, and where it sits in the scrolling list. */
  const [hover, setHover] = useState<{ id: string; top: number; height: number } | null>(null);
  const pressTimer = useRef<number | null>(null);
  /** A long press already handled this touch; the click that follows is not a tap. */
  const pressed = useRef(false);

  useEffect(() => {
    if (!selectedId) return;
    const rows = listRef.current?.querySelectorAll<HTMLElement>('[data-id]') ?? [];
    const row = [...rows].find((el) => el.dataset['id'] === selectedId);
    // scrollIntoView is missing in some test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    row?.scrollIntoView?.({ block: 'nearest' });
  }, [selectedId]);

  useEffect(
    () => () => {
      if (pressTimer.current !== null) window.clearTimeout(pressTimer.current);
    },
    [],
  );

  // Computers: follow the pointer from row to row for the quick actions. The
  // actions float over the row, outside the list, so moving onto them keeps
  // the row; leaving the list hides them.
  const hasRowActions = props.rowActions !== undefined;
  useEffect(() => {
    const root = listRef.current;
    if (!root || phone || !hasRowActions) return;
    const over = (event: Event) => {
      const wrap = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-row-id]') : null;
      const id = wrap?.dataset['rowId'];
      if (!wrap || !id) return;
      const top = wrap.offsetTop;
      const height = wrap.offsetHeight;
      setHover((current) => (current?.id === id && current.top === top ? current : { id, top, height }));
    };
    const leave = () => {
      setHover(null);
    };
    root.addEventListener('mouseover', over);
    root.addEventListener('mouseleave', leave);
    return () => {
      root.removeEventListener('mouseover', over);
      root.removeEventListener('mouseleave', leave);
    };
  }, [phone, hasRowActions, props.status]);

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

  const cancelPress = () => {
    if (pressTimer.current !== null) window.clearTimeout(pressTimer.current);
    pressTimer.current = null;
  };

  const longPress = (id: string) => {
    cancelPress();
    pressed.current = true;
    props.onLongPress?.(id);
  };

  const click = (id: string, event: MouseEvent<HTMLButtonElement>) => {
    if (pressed.current) {
      pressed.current = false;
      return;
    }
    if (event.target instanceof Element && event.target.closest('.row__edge')) {
      props.onToggleRead?.(id);
      return;
    }
    if (phone && selecting) {
      props.onPick?.(id, 'toggle');
      return;
    }
    if (!phone && (event.ctrlKey || event.metaKey)) {
      props.onPick?.(id, 'toggle');
      return;
    }
    if (!phone && event.shiftKey) {
      props.onPick?.(id, 'range');
      return;
    }
    props.onSelect(id);
  };

  const hovered = !phone && props.rowActions && hover ? items.find((m) => m.id === hover.id) : undefined;
  const extraFilter = MORE_FILTERS.find((f) => f.id === props.filter);

  return (
    <section className={`list${selecting ? ' list--selecting' : ''}`} aria-labelledby="list-title">
      <div className="list__head">
        {props.selectionBar ? (
          <>
            <h1 id="list-title" className="visually-hidden">
              {props.title}
            </h1>
            {props.selectionBar}
          </>
        ) : (
          <>
            <div className="list__heading">
              <h1 id="list-title" className="list__title">
                {props.title}
              </h1>
              <span className="muted small list__count">
                {props.status === 'ready' ? `${String(props.total)} ${props.total === 1 ? 'message' : 'messages'}, ` : ''}
                {props.subtitle}
              </span>
              {props.tools && <span className="list__tools">{props.tools}</span>}
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
              <MenuButton
                label={extraFilter ? `More filters: ${extraFilter.label}` : 'More filters'}
                className="tab tab--menu"
                text={extraFilter?.label ?? 'More'}
                active={extraFilter !== undefined}
                align="left"
              >
                {MORE_FILTERS.map((f) => (
                  <MenuItem
                    key={f.id}
                    kind="radio"
                    checked={props.filter === f.id}
                    onSelect={() => {
                      props.onFilter(props.filter === f.id ? 'all' : f.id);
                    }}
                  >
                    {f.label}
                  </MenuItem>
                ))}
              </MenuButton>
            </div>
          </>
        )}
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
            {groups.map((group) =>
              group.items.map((m, i) => {
                const account = accounts.find((a) => a.id === m.accountId);
                const isChecked = checked.has(m.id);
                const name = who(m, outgoing);
                const subject = m.subject || '(no subject)';
                const where = props.folderOf?.(m) ?? null;
                return (
                  <li key={m.id}>
                    {i === 0 && <h2 className="rows__group">{group.label}</h2>}
                    <div className={`row-wrap${isChecked ? ' row-wrap--checked' : ''}`} data-row-id={m.id}>
                      <input
                        type="checkbox"
                        className="row__check"
                        checked={isChecked}
                        aria-label={`Select ${name}: ${subject}`}
                        onChange={() => {
                          props.onPick?.(m.id, 'check');
                        }}
                      />
                      <button
                        type="button"
                        data-id={m.id}
                        className={`row${m.unread ? ' row--unread' : ''}${isChecked ? ' row--checked' : ''}`}
                        aria-current={m.id === selectedId ? 'true' : undefined}
                        draggable={!phone && props.onDragRows !== undefined}
                        onClick={(e) => {
                          click(m.id, e);
                        }}
                        onDoubleClick={() => {
                          if (!phone && !selecting) props.onOpenWindow?.(m.id);
                        }}
                        onDragStart={(e) => {
                          props.onDragRows?.(m.id, e);
                        }}
                        onDragEnd={() => {
                          props.onDragEnd?.();
                        }}
                        onPointerDown={(e) => {
                          pressed.current = false;
                          if (!phone || e.button !== 0 || !props.onLongPress) return;
                          cancelPress();
                          pressTimer.current = window.setTimeout(() => {
                            longPress(m.id);
                          }, LONG_PRESS_MS);
                        }}
                        onPointerUp={cancelPress}
                        onPointerCancel={cancelPress}
                        onPointerLeave={cancelPress}
                        onContextMenu={(e) => {
                          // Android turns a long press into a context menu event.
                          if (!phone || !props.onLongPress) return;
                          e.preventDefault();
                          if (!pressed.current) longPress(m.id);
                        }}
                      >
                        <span
                          className={`row__edge${m.unread ? ' row__edge--unread' : ''}`}
                          aria-hidden="true"
                          title={m.unread ? 'Mark as read' : 'Mark as unread'}
                        />
                        <Avatar
                          name={outgoing ? (m.to[0]?.name ?? m.to[0]?.address ?? '?') : (m.from?.name ?? m.from?.address ?? '?')}
                          address={outgoing ? (m.to[0]?.address ?? '') : (m.from?.address ?? '')}
                        />
                        <span className="row__text">
                          <span className="row__top">
                            {m.unread && <span className="row__unread" aria-label="Unread" />}
                            <span className="row__who">{name}</span>
                            {(m.threadCount ?? 1) > 1 && (
                              <span className="row__count" aria-label={`${String(m.threadCount)} messages in this conversation`}>
                                {m.threadCount}
                              </span>
                            )}
                            {m.pinned && (
                              <span className="row__icon row__icon--pin" aria-label="Pinned">
                                <Icon name="pin" size={14} filled />
                              </span>
                            )}
                            {m.importance === 'high' && (
                              <span className="row__importance row__importance--high" aria-label="High importance">
                                !
                              </span>
                            )}
                            {m.importance === 'low' && (
                              <span className="row__icon row__importance--low" aria-label="Low importance">
                                <Icon name="arrowDown" size={14} />
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
                          <span className="row__subject">{subject}</span>
                          <span className="row__preview">{m.preview}</span>
                          {(where !== null || m.categories.length > 0 || (m.flagged && m.flagDue !== null)) && (
                            <span className="row__tags">
                              {where !== null && <span className="row__folder">In {where}</span>}
                              {m.flagged && m.flagDue !== null && (
                                <span className={`row__due${isOverdue(m.flagDue, now) ? ' row__due--overdue' : ''}`}>
                                  {dueLabel(m.flagDue, now)}
                                </span>
                              )}
                              {m.categories.map((c) => (
                                <span
                                  key={c}
                                  className="chip"
                                  style={{ '--chip': colourOf(props.categories, c) } as CSSProperties}
                                >
                                  {c}
                                </span>
                              ))}
                            </span>
                          )}
                        </span>
                      </button>
                    </div>
                  </li>
                );
              }),
            )}
          </ul>
        )}

        {hovered && hover && props.rowActions && (
          <div
            className="row-actions"
            role="toolbar"
            aria-label={`Actions for ${hovered.subject || '(no subject)'}`}
            style={{ top: hover.top + Math.max(0, hover.height / 2 - 18) }}
          >
            {props.rowActions(hovered)}
          </div>
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
