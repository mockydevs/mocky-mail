import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type SyntheticEvent,
} from 'react';

import type { CalendarEvent } from '../../api/types';
import { atMinutes, byDay, cssColor, DAY, dayKey, dayText, hhmm, minutesInto, timeText, type DayKey } from './calendarTime';
import { layoutDay } from './layout';

/** One column of the grid: a day, or (side by side) one calendar on a day. */
export interface GridColumn {
  key: string;
  day: Date;
  /** Only this calendar's events; null: every calendar. */
  calendarId: string | null;
  /** Column heading in side-by-side mode. */
  label?: string;
  color?: string | null;
}

/** A time range in a column, in minutes from midnight. */
export interface SlotRange {
  column: GridColumn;
  start: number;
  end: number;
  allDay: boolean;
}

interface Props {
  columns: GridColumn[];
  events: readonly CalendarEvent[];
  /** Ticks each minute, for the now-line. */
  now: Date;
  /** The day the keyboard cursor is on. */
  selectedKey: DayKey;
  /** Computers: drag, resize and quick create. Phones: a tap opens the full editor. */
  interactive: boolean;
  /** Events can be added somewhere. */
  canAdd: boolean;
  /** This event may be dragged and resized. */
  canEdit: (e: CalendarEvent) => boolean;
  label: string;
  onOpen: (e: CalendarEvent) => void;
  /** Phones, or "More options": the full editor for this time. */
  onCreate: (range: SlotRange, title: string) => void;
  /** Quick create: save an event with just a title. */
  onQuickSave: (range: SlotRange, title: string) => void;
  /** Dragged or resized (`copy`: Ctrl was held, so a copy goes there instead). */
  onChange: (e: CalendarEvent, start: number, end: number, copy: boolean) => void;
  onDelete: (e: CalendarEvent) => void;
  /** The keyboard cursor moved to another day (it may be outside the view). */
  onSelectDay: (day: Date, step: number) => void;
  /** A day's heading: show that day on its own. */
  onDayHeader?: ((day: Date) => void) | undefined;
}

/** Half-hour rows, 24 px each. */
const SLOT_MINUTES = 30;
const SLOTS = (24 * 60) / SLOT_MINUTES;
export const PX_PER_MINUTE = 24 / SLOT_MINUTES;

type Drag =
  | { kind: 'select'; col: number; from: number; to: number; allDay: boolean }
  | {
      kind: 'move' | 'resize';
      event: CalendarEvent;
      col: number;
      /** Slot where the event was grabbed. */
      grab: number;
      moved: boolean;
      copy: boolean;
    };

interface Preview {
  col: number;
  top: number;
  length: number;
  title: string;
  copy: boolean;
}

const isStrip = (e: CalendarEvent) => e.allDay || e.end - e.start >= DAY;

const headWeekday = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const headWeekdayLong = new Intl.DateTimeFormat(undefined, { weekday: 'long' });
/** "Fri 27" over a column; "Friday 27" when it is the only one. */
const headText = (day: Date, alone: boolean) =>
  `${(alone ? headWeekdayLong : headWeekday).format(day)} ${String(day.getDate())}`;

/** The cell under the pointer: where it is, else what was hit (tests have no layout). */
function cellAt(ev: PointerEvent | ReactPointerEvent): { col: number; slot: number } | null {
  const hit = typeof document.elementFromPoint === 'function' ? document.elementFromPoint(ev.clientX, ev.clientY) : null;
  const target =
    hit?.closest<HTMLElement>('[data-slot]') ?? (ev.target instanceof Element ? ev.target.closest<HTMLElement>('[data-slot]') : null);
  if (!target) return null;
  const col = Number(target.dataset['col']);
  const slot = Number(target.dataset['slot']);
  return Number.isFinite(col) && Number.isFinite(slot) ? { col, slot } : null;
}

/**
 * Day, work week and week views: half-hour rows, an all-day strip on top,
 * overlapping events side by side and a line at the current time. On
 * computers, click or drag over empty time to add an event, drag an event
 * to move it (Ctrl: a copy) and drag its bottom edge to change its end. The
 * arrow keys move between half hours and days, Enter adds or opens, N starts
 * a new event and Delete deletes.
 */
export function TimeGrid({
  columns,
  events,
  now,
  selectedKey,
  interactive,
  canAdd,
  canEdit,
  label,
  onOpen,
  onCreate,
  onQuickSave,
  onChange,
  onDelete,
  onSelectDay,
  onDayHeader,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  /** The window listeners of the drag in progress. */
  const listening = useRef<{ move: (ev: PointerEvent) => void; up: (ev: PointerEvent) => void; esc: (ev: globalThis.KeyboardEvent) => void } | null>(null);
  const suppressClick = useRef(false);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [selecting, setSelecting] = useState<{ col: number; from: number; to: number } | null>(null);
  const [quick, setQuick] = useState<SlotRange | null>(null);
  const [quickTitle, setQuickTitle] = useState('');
  const [slot, setSlot] = useState(18);
  const [calCol, setCalCol] = useState(0);
  const keyNav = useRef(false);
  const columnsRef = useRef(columns);
  useEffect(() => {
    columnsRef.current = columns;
  });

  const sideBySide = columns.some((c) => c.calendarId !== null);
  const dayCol = Math.max(
    0,
    columns.findIndex((c) => dayKey(c.day) === selectedKey),
  );
  const cursorCol = sideBySide ? Math.min(calCol, columns.length - 1) : dayCol;

  // Open at the start of the working day.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = Math.round(7.5 * 60 * PX_PER_MINUTE);
  }, []);

  useEffect(() => {
    if (!keyNav.current) return;
    keyNav.current = false;
    gridRef.current?.querySelector<HTMLElement>(`[data-col="${String(cursorCol)}"][data-slot="${String(slot)}"]`)?.focus();
  }, [cursorCol, slot, selectedKey]);

  const days = useMemo(() => byDay(events), [events]);
  const eventsOf = (c: GridColumn) =>
    (days.get(dayKey(c.day)) ?? []).filter((e) => c.calendarId === null || e.calendarId === c.calendarId);

  const range = (col: number, from: number, to: number, allDay = false): SlotRange | null => {
    const column = columnsRef.current[col];
    if (!column) return null;
    return { column, start: Math.min(from, to) * SLOT_MINUTES, end: (Math.max(from, to) + 1) * SLOT_MINUTES, allDay };
  };

  const create = (r: SlotRange | null) => {
    if (!r || !canAdd) return;
    if (interactive) {
      setQuickTitle('');
      setQuick(r);
    } else {
      onCreate(r, '');
    }
  };

  /** Where a drag would put the event now. */
  const placeFor = (d: Extract<Drag, { kind: 'move' | 'resize' }>, at: { col: number; slot: number }) => {
    const from = columnsRef.current[d.col];
    const to = columnsRef.current[at.col];
    if (!from || !to) return null;
    const startMins = minutesInto(from.day, d.event.start);
    if (d.kind === 'move') {
      const shift = (at.slot - d.grab) * SLOT_MINUTES;
      const start = atMinutes(to.day, startMins + shift);
      return { start, end: start + (d.event.end - d.event.start), col: at.col };
    }
    const end = Math.max(atMinutes(to.day, (at.slot + 1) * SLOT_MINUTES), d.event.start + 15 * 60);
    return { start: d.event.start, end, col: d.col };
  };

  const unlisten = () => {
    const l = listening.current;
    listening.current = null;
    if (!l) return;
    window.removeEventListener('pointermove', l.move);
    window.removeEventListener('pointerup', l.up);
    window.removeEventListener('keydown', l.esc, true);
  };

  const stopDrag = () => {
    unlisten();
    drag.current = null;
    setPreview(null);
    setSelecting(null);
  };

  function onMove(ev: PointerEvent) {
    const d = drag.current;
    if (!d) return;
    const at = cellAt(ev);
    if (!at) return;
    if (d.kind === 'select') {
      if (at.col !== d.col || d.allDay) return;
      d.to = at.slot;
      setSelecting({ col: d.col, from: d.from, to: d.to });
      return;
    }
    if (d.kind === 'move' && at.col === d.col && at.slot === d.grab && !d.moved) return;
    d.moved = true;
    d.copy = ev.ctrlKey || ev.metaKey;
    const place = placeFor(d, at);
    const column = columnsRef.current[place?.col ?? d.col];
    if (!place || !column) return;
    setPreview({
      col: place.col,
      top: Math.max(0, minutesInto(column.day, place.start)),
      length: Math.max(15, Math.round((place.end - place.start) / 60)),
      title: d.event.title,
      copy: d.copy,
    });
  }

  function onUp(ev: PointerEvent) {
    const d = drag.current;
    stopDrag();
    if (!d) return;
    if (d.kind === 'select') {
      create(range(d.col, d.from, d.to, d.allDay));
      return;
    }
    if (!d.moved) return;
    // The click that ends a drag doesn't open the event.
    suppressClick.current = true;
    window.setTimeout(() => {
      suppressClick.current = false;
    }, 0);
    const at = cellAt(ev);
    const place = at ? placeFor(d, at) : null;
    if (!place) return;
    if (place.start === d.event.start && place.end === d.event.end) return;
    onChange(d.event, place.start, place.end, d.copy || ev.ctrlKey || ev.metaKey);
  }

  function onEscape(ev: globalThis.KeyboardEvent) {
    if (ev.key !== 'Escape') return;
    ev.preventDefault();
    ev.stopPropagation();
    suppressClick.current = true;
    window.setTimeout(() => {
      suppressClick.current = false;
    }, 0);
    stopDrag();
  }

  const startDrag = (next: Drag) => {
    unlisten();
    drag.current = next;
    listening.current = { move: onMove, up: onUp, esc: onEscape };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('keydown', onEscape, true);
  };

  useEffect(() => unlisten, []);

  const onCellPointerDown = (ev: ReactPointerEvent, col: number, s: number, allDay = false) => {
    if (!interactive || ev.button !== 0 || !canAdd) return;
    ev.preventDefault();
    setQuick(null);
    startDrag({ kind: 'select', col, from: s, to: s, allDay });
    setSelecting(allDay ? null : { col, from: s, to: s });
  };

  const onBlockPointerDown = (ev: ReactPointerEvent<HTMLElement>, e: CalendarEvent, col: number, top: number, kind: 'move' | 'resize') => {
    if (!interactive || ev.button !== 0 || !canEdit(e)) return;
    ev.stopPropagation();
    const block = ev.currentTarget.closest('.cal-block') ?? ev.currentTarget;
    const rect = block.getBoundingClientRect();
    const grabMinutes = top + Math.max(0, ev.clientY - rect.top) / PX_PER_MINUTE;
    startDrag({
      kind,
      event: e,
      col,
      grab: kind === 'resize' ? Math.floor((top - 1) / SLOT_MINUTES) : Math.floor(grabMinutes / SLOT_MINUTES),
      moved: false,
      copy: ev.ctrlKey || ev.metaKey,
    });
  };

  const onCellKey = (ev: KeyboardEvent<HTMLElement>, col: number, s: number) => {
    const column = columns[col];
    if (!column) return;
    switch (ev.key) {
      case 'ArrowUp':
      case 'ArrowDown': {
        ev.preventDefault();
        keyNav.current = true;
        setSlot(Math.min(SLOTS - 1, Math.max(0, s + (ev.key === 'ArrowUp' ? -1 : 1))));
        return;
      }
      case 'ArrowLeft':
      case 'ArrowRight': {
        ev.preventDefault();
        keyNav.current = true;
        setSlot(s);
        const step = ev.key === 'ArrowLeft' ? -1 : 1;
        if (sideBySide) {
          const next = col + step;
          if (next >= 0 && next < columns.length) {
            setCalCol(next);
            return;
          }
          setCalCol(step < 0 ? columns.length - 1 : 0);
        }
        const d = new Date(column.day.getFullYear(), column.day.getMonth(), column.day.getDate() + step);
        onSelectDay(d, step);
        return;
      }
      case 'Enter':
      case ' ':
        ev.preventDefault();
        create(range(col, s, s));
        return;
      case 'n':
      case 'N':
        if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
        ev.preventDefault();
        ev.stopPropagation();
        if (canAdd) onCreate({ column, start: s * SLOT_MINUTES, end: s * SLOT_MINUTES + 60, allDay: false }, '');
        return;
      default:
    }
  };

  const onBlockKey = (ev: KeyboardEvent<HTMLElement>, e: CalendarEvent) => {
    if (ev.key === 'Delete' || ev.key === 'Backspace') {
      ev.preventDefault();
      onDelete(e);
    }
  };

  const openBlock = (e: CalendarEvent) => {
    if (suppressClick.current) return;
    setQuick(null);
    onOpen(e);
  };

  const submitQuick = (ev: SyntheticEvent) => {
    ev.preventDefault();
    if (!quick) return;
    const r = quick;
    setQuick(null);
    onQuickSave(r, quickTitle);
  };

  const quickWhen = (r: SlotRange) =>
    r.allDay ? `${dayText(r.column.day)}, all day` : `${dayText(r.column.day)}, ${hhmm(r.start)} – ${hhmm(r.end % (24 * 60))}`;

  const nowMinutes = minutesInto(new Date(now.getFullYear(), now.getMonth(), now.getDate()), Math.floor(now.getTime() / 1000));
  const todayKey = dayKey(now);
  const template = `repeat(${String(columns.length)}, minmax(0, 1fr))`;

  const quickForm = (r: SlotRange) => (
    <form
      className={`quick-create${columns.length > 2 && columns.indexOf(r.column) >= columns.length / 2 ? ' quick-create--right' : ''}`}
      aria-label="Quick event"
      style={r.allDay ? undefined : { top: r.start * PX_PER_MINUTE }}
      onSubmit={submitQuick}
      onPointerDown={(ev) => {
        ev.stopPropagation();
      }}
    >
      <p className="quick-create__when">{quickWhen(r)}</p>
      <input
        className="field"
        aria-label="Title"
        placeholder="Add a title"
        value={quickTitle}
        maxLength={500}
        // Opened by the user's click to type the title straight away.
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus
        onChange={(ev) => {
          setQuickTitle(ev.target.value);
        }}
        onKeyDown={(ev) => {
          if (ev.key === 'Escape') {
            ev.preventDefault();
            ev.stopPropagation();
            setQuick(null);
          }
        }}
      />
      <div className="quick-create__actions">
        <button
          type="button"
          className="btn btn--link btn--small"
          onClick={() => {
            setQuick(null);
            onCreate(r, quickTitle);
          }}
        >
          More options
        </button>
        <button type="submit" className="btn btn--primary btn--small">
          Save
        </button>
      </div>
    </form>
  );

  const chip = (e: CalendarEvent) => (
    <button
      key={`${e.id}:${String(e.start)}`}
      type="button"
      className={`cal-event cal-event--chip cal-event--allday${e.showAs === 'tentative' ? ' cal-event--tentative' : ''}`}
      style={{ ['--event-color' as string]: cssColor(e.color) }}
      onClick={() => {
        openBlock(e);
      }}
      onKeyDown={(ev) => {
        onBlockKey(ev, e);
      }}
    >
      <span className="cal-event__title">{e.title || '(No title)'}</span>
    </button>
  );

  return (
    <div className={`tgrid${preview ? ' tgrid--dragging' : ''}`} style={{ ['--tgrid-cols' as string]: template }}>
      <div className="tgrid__head">
        <span className="tgrid__corner" aria-hidden="true" />
        <div className="tgrid__heads">
          {columns.map((c) => {
            const text = c.label ?? headText(c.day, columns.length === 1);
            const today = dayKey(c.day) === todayKey;
            return (
              <div key={c.key} className={`tgrid__heading${today ? ' tgrid__heading--today' : ''}`}>
                {c.label !== undefined && (
                  <span className="tgrid__dot" style={{ background: cssColor(c.color ?? null) }} aria-hidden="true" />
                )}
                {onDayHeader && c.label === undefined ? (
                  <button
                    type="button"
                    className="tgrid__day-btn"
                    aria-label={`Show ${dayText(c.day)}`}
                    onClick={() => {
                      onDayHeader(c.day);
                    }}
                  >
                    {text}
                  </button>
                ) : (
                  <span>{text}</span>
                )}
              </div>
            );
          })}
        </div>
      </div>
      <div className="tgrid__allday" role="group" aria-label="All day">
        <span className="tgrid__corner tgrid__corner--label">All day</span>
        <div className="tgrid__heads">
          {columns.map((c, col) => {
            const list = eventsOf(c).filter(isStrip);
            return (
              // Keyboard users add all-day events with N and the editor.
              // eslint-disable-next-line jsx-a11y/no-static-element-interactions, jsx-a11y/click-events-have-key-events
              <div
                key={c.key}
                className="tgrid__allday-cell"
                data-col={col}
                aria-label={`${c.label ? `${c.label}, ` : ''}${dayText(c.day)}, all day`}
                onPointerDown={(ev) => {
                  if (ev.target === ev.currentTarget) onCellPointerDown(ev, col, 0, true);
                }}
                onClick={(ev) => {
                  if (!interactive && ev.target === ev.currentTarget) create({ column: c, start: 0, end: 24 * 60, allDay: true });
                }}
              >
                {list.map(chip)}
                {quick?.allDay && quick.column.key === c.key && quickForm(quick)}
              </div>
            );
          })}
        </div>
      </div>
      <div className="tgrid__body" ref={scrollRef}>
        <div className="tgrid__inner">
          <div ref={gridRef} role="grid" aria-label={label} className="tgrid__rows">
            {Array.from({ length: SLOTS }, (_, s) => (
              <div key={s} role="row" className={`tgrid__row${s % 2 === 0 ? ' tgrid__row--hour' : ''}`}>
                <div role="rowheader" className="tgrid__time">
                  {s % 2 === 0 && s > 0 ? hhmm(s * SLOT_MINUTES) : <span className="visually-hidden">{hhmm(s * SLOT_MINUTES)}</span>}
                </div>
                {columns.map((c, col) => {
                  const chosen =
                    selecting?.col === col && s >= Math.min(selecting.from, selecting.to) && s <= Math.max(selecting.from, selecting.to);
                  return (
                    <div
                      key={c.key}
                      role="gridcell"
                      tabIndex={col === cursorCol && s === slot ? 0 : -1}
                      aria-label={`${c.label ? `${c.label}, ` : ''}${dayText(c.day)}, ${hhmm(s * SLOT_MINUTES)}`}
                      className={`tgrid__cell${chosen ? ' tgrid__cell--chosen' : ''}${dayKey(c.day) === todayKey ? ' tgrid__cell--today' : ''}`}
                      data-col={col}
                      data-slot={s}
                      onPointerDown={(ev) => {
                        onCellPointerDown(ev, col, s);
                      }}
                      onClick={() => {
                        if (!interactive) create(range(col, s, s));
                      }}
                      onFocus={() => {
                        setSlot(s);
                        if (sideBySide) setCalCol(col);
                      }}
                      onKeyDown={(ev) => {
                        onCellKey(ev, col, s);
                      }}
                    />
                  );
                })}
              </div>
            ))}
          </div>
          <div className="tgrid__overlay">
            <span className="tgrid__gutter" />
            <div className="tgrid__cols">
              {columns.map((c, col) => {
                const placed = layoutDay(c.day, eventsOf(c).filter((e) => !isStrip(e)));
                return (
                  <div key={c.key} className="tgrid__col" data-col-events={col}>
                    {placed.map((p) => {
                      const e = p.event;
                      const editable = interactive && canEdit(e);
                      return (
                        <div
                          key={`${e.id}:${String(e.start)}`}
                          className={[
                            'cal-block',
                            e.showAs === 'tentative' ? 'cal-block--tentative' : '',
                            e.showAs === 'free' ? 'cal-block--free' : '',
                            p.length < 30 ? 'cal-block--short' : '',
                          ]
                            .filter(Boolean)
                            .join(' ')}
                          style={{
                            ['--event-color' as string]: cssColor(e.color),
                            top: p.top * PX_PER_MINUTE,
                            height: p.length * PX_PER_MINUTE - 2,
                            left: `calc(${String((p.column / p.columns) * 100)}% + 1px)`,
                            width: `calc(${String(100 / p.columns)}% - 3px)`,
                          }}
                          data-column={p.column}
                          data-columns={p.columns}
                        >
                          <button
                            type="button"
                            className="cal-block__main"
                            aria-label={`${e.title || '(No title)'}, ${timeText(e.start)} to ${timeText(e.end)}${e.location ? `, ${e.location}` : ''}`}
                            onPointerDown={(ev) => {
                              onBlockPointerDown(ev, e, col, p.top, 'move');
                            }}
                            onClick={() => {
                              openBlock(e);
                            }}
                            onKeyDown={(ev) => {
                              onBlockKey(ev, e);
                            }}
                          >
                            <span className="cal-block__title">{e.title || '(No title)'}</span>
                            <span className="cal-block__time">
                              {timeText(e.start)} – {timeText(e.end)}
                            </span>
                            {e.location && <span className="cal-block__place">{e.location}</span>}
                          </button>
                          {editable && !p.continuesAfter && (
                            <div
                              className="cal-block__resize"
                              data-testid="resize"
                              aria-hidden="true"
                              onPointerDown={(ev) => {
                                onBlockPointerDown(ev, e, col, p.top + p.length, 'resize');
                              }}
                            />
                          )}
                        </div>
                      );
                    })}
                    {preview?.col === col && (
                      <div
                        className={`cal-block cal-block--ghost${preview.copy ? ' cal-block--copy' : ''}`}
                        style={{ top: preview.top * PX_PER_MINUTE, height: preview.length * PX_PER_MINUTE - 2, left: 1, right: 2 }}
                        role="status"
                        aria-label={preview.copy ? 'Copy here' : 'Move here'}
                      >
                        <span className="cal-block__title">{preview.title || '(No title)'}</span>
                      </div>
                    )}
                    {dayKey(c.day) === todayKey && (
                      <div
                        className="tgrid__now"
                        style={{ top: nowMinutes * PX_PER_MINUTE }}
                        role="img"
                        aria-label={`Now, ${timeText(Math.floor(now.getTime() / 1000))}`}
                      />
                    )}
                    {quick && !quick.allDay && quick.column.key === c.key && quickForm(quick)}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
