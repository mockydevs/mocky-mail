import { useEffect, useId, useMemo, useRef, useState, type ReactNode, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, CalendarEvent, CalendarInfo, CalendarStatus, EventInput } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { Modal } from '../../ui/Modal';
import {
  addDays,
  addMonths,
  byDay,
  cssColor,
  DAY,
  dayKey,
  dayText,
  deviceZone,
  eventDays,
  fromDayKey,
  fromInputs,
  monthGrid,
  reminderText,
  repeatText,
  seconds,
  startOfDay,
  timeText,
  timeValue,
  utcMidnight,
  whenText,
  type DayKey,
} from './calendarTime';

interface Props {
  accounts: readonly Account[];
  /** Phone layout: the agenda, and details or the editor fill the screen. */
  phone: boolean;
  onClose: () => void;
  /** Start a message to these people. */
  onWrite: (to: string) => void;
  /** Reminder for new events, minutes before; -1 for none. */
  defaultReminder: number;
  /** Today, for tests. */
  today?: Date;
}

/** Reminder choices in the editor, minutes before (-1: none). */
const REMINDERS = [-1, 0, 5, 10, 15, 30, 60, 120, 1440, 10080];

type Mode = 'month' | 'agenda';

/** What the editor holds: dates and times as the inputs show them. */
interface Form {
  id?: string;
  calendarId: string;
  title: string;
  location: string;
  notes: string;
  allDay: boolean;
  startDay: DayKey;
  startTime: string;
  endDay: DayKey;
  endTime: string;
  repeat: EventInput['repeat'];
  /** The series had a rule the editor can't show. */
  customRule: boolean;
  /** Minutes before (-1: none), or 'keep' for reminders set elsewhere. */
  reminder: number | 'keep';
}

type Pane = { kind: 'day' } | { kind: 'view'; event: CalendarEvent } | { kind: 'edit'; form: Form; series: boolean };

const MONTH = new Intl.DateTimeFormat(undefined, {
  month: 'long',
  year: 'numeric',
});
const WEEKDAYS = Array.from({ length: 7 }, (_, i) =>
  new Intl.DateTimeFormat(undefined, { weekday: 'short' }).format(new Date(2024, 0, 1 + i)),
);

function formFrom(e: CalendarEvent): Form {
  const [first, last] = eventDays(e);
  return {
    id: e.id,
    calendarId: e.calendarId,
    title: e.title,
    location: e.location,
    notes: e.notes,
    allDay: e.allDay,
    startDay: dayKey(first),
    startTime: e.allDay ? '09:00' : timeValue(new Date(e.start * 1000)),
    endDay: dayKey(last),
    endTime: e.allDay ? '10:00' : timeValue(new Date(e.end * 1000)),
    repeat: e.repeat === 'custom' ? 'keep' : e.repeat,
    customRule: e.repeat === 'custom',
    reminder: reminderChoice(e.reminders),
  };
}

/** An event's reminders as one editor choice, or 'keep' when it can't show them. */
function reminderChoice(reminders: readonly number[]): Form['reminder'] {
  const [only, ...rest] = reminders;
  if (only === undefined) return -1;
  return rest.length === 0 && REMINDERS.includes(only) ? only : 'keep';
}

function newForm(day: Date, calendarId: string, now: Date, reminder: number): Form {
  // Today: the next whole hour; another day: 9:00.
  const isToday = dayKey(day) === dayKey(now);
  const hour = isToday ? Math.min(now.getHours() + 1, 23) : 9;
  const pad = (n: number) => String(n).padStart(2, '0');
  return {
    calendarId,
    title: '',
    location: '',
    notes: '',
    allDay: false,
    startDay: dayKey(day),
    startTime: `${pad(hour)}:00`,
    endDay: dayKey(day),
    endTime: `${pad(Math.min(hour + 1, 23))}:${hour === 23 ? '59' : '00'}`,
    repeat: 'none',
    customRule: false,
    reminder,
  };
}

function toInput(form: Form): EventInput | string {
  const start = form.allDay ? utcMidnight(form.startDay) : fromInputs(form.startDay, form.startTime);
  const end = form.allDay ? utcMidnight(form.endDay) + DAY : fromInputs(form.endDay, form.endTime);
  if (!form.startDay || !form.endDay) return 'Choose a date for the event.';
  if (end < start || (form.allDay && end <= start)) return 'The event ends before it starts.';
  return {
    ...(form.id ? { id: form.id } : {}),
    calendarId: form.calendarId,
    title: form.title,
    location: form.location,
    notes: form.notes,
    allDay: form.allDay,
    start,
    end,
    repeat: form.repeat,
    timeZone: deviceZone(),
    reminders: form.reminder === 'keep' ? null : form.reminder < 0 ? [] : [form.reminder],
  };
}

/**
 * The calendar: each account's calendars on its mail server (CalDAV, for
 * example SOGo), kept on the device so they show offline. A month view and
 * an agenda; add, edit and delete events (one date of a series, or all).
 */
export function CalendarView({ accounts, phone, onClose, onWrite, defaultReminder, today }: Props) {
  const api = useMailApi();
  const id = useId();
  const now = useMemo(() => today ?? new Date(), [today]);
  const [mode, setMode] = useState<Mode>(phone ? 'agenda' : 'month');
  const [month, setMonth] = useState(() => new Date(now.getFullYear(), now.getMonth(), 1));
  const [selected, setSelected] = useState<Date>(() => startOfDay(now));
  const [calendars, setCalendars] = useState<CalendarInfo[]>([]);
  const [statuses, setStatuses] = useState<CalendarStatus[]>([]);
  const [events, setEvents] = useState<CalendarEvent[] | null>(null);
  const [revision, setRevision] = useState(0);
  const [pane, setPane] = useState<Pane>({ kind: 'day' });
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncing, setSyncing] = useState(true);
  const [deleting, setDeleting] = useState<CalendarEvent | null>(null);

  // Escape (and the phone's Back) closes an event or the editor first.
  const root = useRef<HTMLElement>(null);
  const open = useRef(false);
  useEffect(() => {
    open.current = pane.kind !== 'day' && deleting === null;
  });
  useEffect(() => {
    const section = root.current;
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape' || !open.current) return;
      ev.preventDefault();
      ev.stopPropagation();
      setError(null);
      setPane({ kind: 'day' });
    };
    section?.addEventListener('keydown', onKey);
    return () => {
      section?.removeEventListener('keydown', onKey);
    };
  }, []);

  const grid = useMemo(() => monthGrid(month), [month]);
  const first = grid[0] ?? month;
  const last = grid[grid.length - 1] ?? month;

  useEffect(() => {
    let live = true;
    const from = seconds(mode === 'month' ? first : month);
    const to = seconds(mode === 'month' ? addDays(last, 1) : addMonths(month, 1));
    api.listEvents(from, to).then(
      (list) => {
        if (live) setEvents(list);
      },
      (e: unknown) => {
        if (live) setError(toAppError(e));
      },
    );
    api.listCalendars().then(
      (list) => {
        if (live) setCalendars(list);
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api, mode, month, first, last, revision]);

  // Sync when opened; refresh whenever a sync or a change finishes.
  useEffect(() => {
    const off = api.on('mail://calendar-changed', ({ calendars: next }) => {
      if (next.length > 0) setStatuses(next);
      setRevision((r) => r + 1);
    });
    api.syncCalendars().then(
      (next) => {
        setStatuses(next);
        setSyncing(false);
        setRevision((r) => r + 1);
      },
      () => {
        setSyncing(false);
      },
    );
    return off;
  }, [api]);

  const days = useMemo(() => byDay(events ?? []), [events]);
  const calendarName = (calendarId: string) => calendars.find((c) => c.id === calendarId)?.name ?? 'Calendar';
  const writable = calendars.filter((c) => accounts.some((a) => a.id === c.accountId));

  const startNew = (day: Date) => {
    const target = writable[0];
    if (!target) return;
    setError(null);
    setPane({
      kind: 'edit',
      form: newForm(day, target.id, now, defaultReminder),
      series: false,
    });
  };

  const openEditor = (e: CalendarEvent) => {
    setError(null);
    if (!e.recurring) {
      setPane({ kind: 'edit', form: formFrom(e), series: false });
      return;
    }
    // A series is edited from its first date.
    api.getEvent(e.id).then(
      (master) => {
        setPane({ kind: 'edit', form: formFrom(master), series: true });
      },
      (err: unknown) => {
        setError(toAppError(err));
      },
    );
  };

  const save = (form: Form) => {
    const input = toInput(form);
    if (typeof input === 'string') {
      setError({ kind: 'invalid', message: input });
      return;
    }
    setBusy(true);
    setError(null);
    api.saveEvent(input).then(
      (saved) => {
        setBusy(false);
        setSelected(eventDays(saved)[0]);
        setPane({ kind: 'view', event: saved });
        setRevision((r) => r + 1);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  const remove = (e: CalendarEvent, onlyThisDate: boolean) => {
    setDeleting(null);
    setBusy(true);
    // Without a date to remove, "only this date" must never reach the
    // backend as null, which means the whole series.
    if (onlyThisDate && e.recurrenceId === null) {
      setBusy(false);
      setError({ kind: 'invalid', message: 'Open the date you want to remove from the calendar, then delete it there.' });
      return;
    }
    api.deleteEvent(e.id, onlyThisDate ? e.recurrenceId : null).then(
      () => {
        setBusy(false);
        setPane({ kind: 'day' });
        setRevision((r) => r + 1);
      },
      (err: unknown) => {
        setBusy(false);
        setError(toAppError(err));
      },
    );
  };

  const goMonth = (n: number) => {
    setMonth((m) => addMonths(m, n));
    setPane({ kind: 'day' });
  };

  const selectedKey = dayKey(selected);
  const todayKey = dayKey(now);
  const notes = statuses.filter((s) => s.state !== 'synced');
  const showMain = !phone || pane.kind === 'day';
  const showSide = !phone || pane.kind !== 'day';

  const eventButton = (e: CalendarEvent, compact: boolean) => (
    <button
      key={`${e.id}:${String(e.start)}`}
      type="button"
      className={`cal-event${compact ? ' cal-event--chip' : ''}${e.allDay ? ' cal-event--allday' : ''}`}
      style={{ ['--event-color' as string]: cssColor(e.color) }}
      onClick={(ev) => {
        ev.stopPropagation();
        setError(null);
        setSelected(eventDays(e)[0]);
        setPane({ kind: 'view', event: e });
      }}
    >
      {!e.allDay && <span className="cal-event__time">{timeText(e.start)}</span>}
      <span className="cal-event__title">{e.title || '(No title)'}</span>
      {!compact && e.location && <span className="cal-event__place">{e.location}</span>}
    </button>
  );

  return (
    <section ref={root} className="calendar" aria-labelledby={`${id}-title`}>
      <header className="calendar__head">
        <h1 id={`${id}-title`} className="calendar__title">
          {MONTH.format(month)}
        </h1>
        <div className="calendar__nav">
          <button
            type="button"
            className="icon-btn"
            aria-label="Previous month"
            onClick={() => {
              goMonth(-1);
            }}
          >
            <Icon name="left" />
          </button>
          <button
            type="button"
            className="btn btn--secondary btn--small"
            onClick={() => {
              setMonth(new Date(now.getFullYear(), now.getMonth(), 1));
              setSelected(startOfDay(now));
              setPane({ kind: 'day' });
            }}
          >
            Today
          </button>
          <button
            type="button"
            className="icon-btn"
            aria-label="Next month"
            onClick={() => {
              goMonth(1);
            }}
          >
            <Icon name="right" />
          </button>
        </div>
        <div className="segmented" role="group" aria-label="View">
          {(['month', 'agenda'] as const).map((m) => (
            <button
              key={m}
              type="button"
              className={`segmented__item${mode === m ? ' segmented__item--on' : ''}`}
              aria-pressed={mode === m}
              onClick={() => {
                setMode(m);
              }}
            >
              {m === 'month' ? 'Month' : 'Agenda'}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="icon-btn calendar__sync"
          aria-label="Sync calendars"
          title="Sync calendars"
          disabled={syncing}
          onClick={() => {
            setSyncing(true);
            api.syncCalendars().then(
              (next) => {
                setStatuses(next);
                setSyncing(false);
                setRevision((r) => r + 1);
              },
              (e: unknown) => {
                setError(toAppError(e));
                setSyncing(false);
              },
            );
          }}
        >
          <Icon name="sync" />
        </button>
        <button
          type="button"
          className="btn btn--primary calendar__new"
          disabled={writable.length === 0}
          title={writable.length === 0 ? 'No calendar to add events to yet' : undefined}
          onClick={() => {
            startNew(selected);
          }}
        >
          <Icon name="plus" />
          {phone ? 'New' : 'New event'}
        </button>
        <button type="button" className="icon-btn calendar__close" aria-label="Close calendar" onClick={onClose}>
          <Icon name="close" />
        </button>
      </header>

      {notes.length > 0 && (
        <ul className="calendar__notes" aria-label="Calendar sync">
          {notes.map((s) => (
            <li key={s.accountId} className="muted small">
              {accounts.find((a) => a.id === s.accountId)?.email ?? 'An account'}: {s.message ?? 'Not synced.'}
            </li>
          ))}
        </ul>
      )}

      <div className="calendar__body">
        {showMain && (
          <div className="calendar__main">
            {events === null ? (
              <p className="muted calendar__empty" role="status">
                Loading events…
              </p>
            ) : mode === 'month' ? (
              <div className="month" role="grid" aria-label={MONTH.format(month)}>
                <div className="month__row month__row--head" role="row">
                  {WEEKDAYS.map((w) => (
                    <span key={w} className="month__weekday" role="columnheader">
                      {w}
                    </span>
                  ))}
                </div>
                {Array.from({ length: 6 }, (_, week) => (
                  <div key={week} className="month__row" role="row">
                    {grid.slice(week * 7, week * 7 + 7).map((day) => {
                      const key = dayKey(day);
                      const list = days.get(key) ?? [];
                      const shown = list.slice(0, 3);
                      return (
                        <div
                          key={key}
                          role="gridcell"
                          aria-selected={key === selectedKey}
                          aria-label={`${dayText(day)}, ${list.length === 0 ? 'no events' : `${String(list.length)} event${list.length === 1 ? '' : 's'}`}`}
                          tabIndex={key === selectedKey ? 0 : -1}
                          className={[
                            'month__day',
                            day.getMonth() === month.getMonth() ? '' : 'month__day--other',
                            key === todayKey ? 'month__day--today' : '',
                            key === selectedKey ? 'month__day--selected' : '',
                          ]
                            .filter(Boolean)
                            .join(' ')}
                          onClick={() => {
                            setSelected(day);
                            setPane({ kind: 'day' });
                          }}
                          onDoubleClick={() => {
                            startNew(day);
                          }}
                          onKeyDown={(ev) => {
                            const move = {
                              ArrowLeft: -1,
                              ArrowRight: 1,
                              ArrowUp: -7,
                              ArrowDown: 7,
                            }[ev.key];
                            if (move !== undefined) {
                              ev.preventDefault();
                              const next = addDays(day, move);
                              setSelected(next);
                              if (next.getMonth() !== month.getMonth())
                                setMonth(new Date(next.getFullYear(), next.getMonth(), 1));
                              requestAnimationFrame(() => {
                                document.querySelector<HTMLElement>('.month__day[tabindex="0"]')?.focus();
                              });
                            } else if (ev.key === 'Enter') {
                              setSelected(day);
                              setPane({ kind: 'day' });
                            }
                          }}
                        >
                          <span className="month__date">{day.getDate()}</span>
                          <span className="month__events">
                            {shown.map((e) => eventButton(e, true))}
                            {list.length > shown.length && (
                              <span className="month__more">+{list.length - shown.length} more</span>
                            )}
                          </span>
                        </div>
                      );
                    })}
                  </div>
                ))}
              </div>
            ) : (
              <Agenda days={days} month={month} todayKey={todayKey} render={(e) => eventButton(e, false)} />
            )}
          </div>
        )}

        {showSide && (
          <aside className="calendar__side" aria-label="Event details">
            {error && (
              <p className="form-error" role="alert">
                {error.message}
              </p>
            )}
            {pane.kind === 'day' && (
              <DayList
                day={selected}
                events={days.get(selectedKey) ?? []}
                canAdd={writable.length > 0}
                render={(e) => eventButton(e, false)}
                onAdd={() => {
                  startNew(selected);
                }}
              />
            )}
            {pane.kind === 'view' && (
              <EventCard
                event={pane.event}
                calendarName={calendarName(pane.event.calendarId)}
                busy={busy}
                onBack={() => {
                  setPane({ kind: 'day' });
                }}
                onEdit={() => {
                  openEditor(pane.event);
                }}
                onDelete={() => {
                  if (pane.event.recurring) setDeleting(pane.event);
                  else remove(pane.event, false);
                }}
                onWrite={onWrite}
              />
            )}
            {pane.kind === 'edit' && (
              <EventEditor
                key={pane.form.id ?? 'new'}
                initial={pane.form}
                series={pane.series}
                calendars={writable}
                accounts={accounts}
                busy={busy}
                onCancel={() => {
                  setError(null);
                  setPane({ kind: 'day' });
                }}
                onSave={save}
              />
            )}
          </aside>
        )}
      </div>

      {deleting && (
        <Modal
          title="Delete a repeating event"
          description={
            deleting.recurrenceId === null
              ? `“${deleting.title || '(No title)'}” repeats. This deletes every date. To remove one date, open that date in the calendar first.`
              : `“${deleting.title || '(No title)'}” repeats. Delete only ${dayText(eventDays(deleting)[0])}, or every date?`
          }
          size="small"
          onClose={() => {
            setDeleting(null);
          }}
        >
          <div className="modal__actions">
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setDeleting(null);
              }}
            >
              Cancel
            </button>
            {deleting.recurrenceId !== null && (
              <button
                type="button"
                className="btn btn--secondary"
                onClick={() => {
                  remove(deleting, true);
                }}
              >
                Only this date
              </button>
            )}
            <button
              type="button"
              className="btn btn--danger"
              onClick={() => {
                remove(deleting, false);
              }}
            >
              Every date
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}

function Agenda({
  days,
  month,
  todayKey,
  render,
}: {
  days: Map<DayKey, CalendarEvent[]>;
  month: Date;
  todayKey: DayKey;
  render: (e: CalendarEvent) => ReactNode;
}) {
  const keys = [...days.keys()]
    .filter((k) => {
      const d = fromDayKey(k);
      return d.getMonth() === month.getMonth() && d.getFullYear() === month.getFullYear();
    })
    // This month: from today on.
    .filter((k) => k >= todayKey || todayKey.slice(0, 7) !== k.slice(0, 7))
    .sort();
  if (keys.length === 0) return <p className="muted calendar__empty">Nothing on the calendar this month.</p>;
  return (
    <ol className="agenda" aria-label="Agenda">
      {keys.map((k) => (
        <li key={k} className={`agenda__day${k === todayKey ? ' agenda__day--today' : ''}`}>
          <h2 className="agenda__date">
            {k === todayKey ? 'Today · ' : ''}
            {dayText(fromDayKey(k))}
          </h2>
          <div className="agenda__events">{(days.get(k) ?? []).map(render)}</div>
        </li>
      ))}
    </ol>
  );
}

function DayList({
  day,
  events,
  canAdd,
  render,
  onAdd,
}: {
  day: Date;
  events: CalendarEvent[];
  canAdd: boolean;
  render: (e: CalendarEvent) => ReactNode;
  onAdd: () => void;
}) {
  return (
    <div className="day-list">
      <h2 className="day-list__title">{dayText(day)}</h2>
      {events.length === 0 ? (
        <p className="muted">Nothing on this day.</p>
      ) : (
        <div className="day-list__events">{events.map(render)}</div>
      )}
      {canAdd && (
        <button type="button" className="btn btn--secondary" onClick={onAdd}>
          <Icon name="plus" />
          Add an event on this day
        </button>
      )}
    </div>
  );
}

const STATUS: Record<string, string> = {
  accepted: 'Accepted',
  declined: 'Declined',
  tentative: 'Maybe',
  'needs-action': 'No answer yet',
};

function EventCard({
  event,
  calendarName,
  busy,
  onBack,
  onEdit,
  onDelete,
  onWrite,
}: {
  event: CalendarEvent;
  calendarName: string;
  busy: boolean;
  onBack: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onWrite: (to: string) => void;
}) {
  const people = [...(event.organizer ? [event.organizer] : []), ...event.attendees].filter(
    (p, i, all) => all.findIndex((q) => q.email.toLowerCase() === p.email.toLowerCase()) === i,
  );
  return (
    <article className="event-card" aria-label={event.title || '(No title)'}>
      <button type="button" className="phone-back event-card__back" onClick={onBack}>
        <Icon name="back" />
        Back
      </button>
      <h2 className="event-card__title">
        <span className="event-card__dot" style={{ background: cssColor(event.color) }} aria-hidden="true" />
        {event.title || '(No title)'}
      </h2>
      <p className="event-card__line">
        <Icon name="clock" size={16} />
        {whenText(event)}
      </p>
      {event.recurring && (
        <p className="event-card__line">
          <Icon name="repeat" size={16} />
          {repeatText(event.repeat, new Date(event.start * 1000))}
        </p>
      )}
      {event.location && (
        <p className="event-card__line">
          <Icon name="pin" size={16} />
          {event.location}
        </p>
      )}
      {event.reminders.length > 0 && (
        <p className="event-card__line">
          <Icon name="bell" size={16} />
          {event.reminders.map(reminderText).join(', ')}
        </p>
      )}
      <p className="event-card__line muted">
        <Icon name="calendar" size={16} />
        {calendarName}
        {event.tentative ? ' · Tentative' : ''}
      </p>
      {event.notes && <p className="event-card__notes">{event.notes}</p>}
      {people.length > 0 && (
        <>
          <h3 className="event-card__subhead">People</h3>
          <ul className="event-card__people">
            {people.map((p) => (
              <li key={p.email}>
                <span>
                  {p.name ?? p.email}
                  {event.organizer?.email.toLowerCase() === p.email.toLowerCase() ? ' (organiser)' : ''}
                </span>
                <span className="muted small">{STATUS[p.status ?? ''] ?? ''}</span>
              </li>
            ))}
          </ul>
          <button
            type="button"
            className="btn btn--secondary btn--small"
            onClick={() => {
              onWrite(people.map((p) => p.email).join(', '));
            }}
          >
            Email everyone
          </button>
        </>
      )}
      <div className="event-card__actions">
        <button type="button" className="btn btn--primary" disabled={busy} onClick={onEdit}>
          Edit
        </button>
        <button type="button" className="btn btn--danger" disabled={busy} onClick={onDelete}>
          Delete
        </button>
      </div>
    </article>
  );
}

function EventEditor({
  initial,
  series,
  calendars,
  accounts,
  busy,
  onCancel,
  onSave,
}: {
  initial: Form;
  series: boolean;
  calendars: CalendarInfo[];
  accounts: readonly Account[];
  busy: boolean;
  onCancel: () => void;
  onSave: (form: Form) => void;
}) {
  const id = useId();
  const [form, setForm] = useState(initial);
  const set = <K extends keyof Form>(key: K, value: Form[K]) => {
    setForm((f) => ({ ...f, [key]: value }));
  };
  const start = fromDayKey(form.startDay);
  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    onSave(form);
  };
  const label = (c: CalendarInfo) => {
    const email = accounts.find((a) => a.id === c.accountId)?.email;
    return calendars.filter((x) => x.accountId === c.accountId).length > 1 || !email
      ? `${c.name} (${email ?? ''})`
      : email;
  };
  return (
    <form className="event-editor" aria-label={form.id ? 'Edit event' : 'New event'} onSubmit={submit}>
      <h2 className="event-editor__title">{form.id ? 'Edit event' : 'New event'}</h2>
      {series && <p className="form-hint">This event repeats. Changes apply to every date.</p>}
      <label className="form-field">
        <span className="form-field__label">Title</span>
        <input
          className="field"
          value={form.title}
          maxLength={500}
          // The first field of a form the user just opened.
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
          onChange={(e) => {
            set('title', e.target.value);
          }}
        />
      </label>
      {calendars.length > 1 && (
        <label className="form-field">
          <span className="form-field__label">Calendar</span>
          <select
            className="field"
            value={form.calendarId}
            onChange={(e) => {
              set('calendarId', e.target.value);
            }}
          >
            {calendars.map((c) => (
              <option key={c.id} value={c.id}>
                {label(c)}
              </option>
            ))}
          </select>
        </label>
      )}
      <label className="choice">
        <input
          type="checkbox"
          checked={form.allDay}
          onChange={(e) => {
            set('allDay', e.target.checked);
          }}
        />
        All day
      </label>
      <div className="event-editor__times">
        <label className="form-field">
          <span className="form-field__label">Starts</span>
          <input
            className="field"
            type="date"
            required
            value={form.startDay}
            onChange={(e) => {
              const next = e.target.value;
              // Moving the start moves the end with it.
              const shift = (fromDayKey(next).getTime() - fromDayKey(form.startDay).getTime()) / 86_400_000;
              setForm((f) => ({
                ...f,
                startDay: next,
                endDay: next && f.endDay ? dayKey(addDays(fromDayKey(f.endDay), Math.round(shift))) : next,
              }));
            }}
          />
        </label>
        {!form.allDay && (
          <label className="form-field">
            <span className="form-field__label">
              <span className="visually-hidden">Start </span>Time
            </span>
            <input
              className="field"
              type="time"
              required
              value={form.startTime}
              onChange={(e) => {
                set('startTime', e.target.value);
              }}
            />
          </label>
        )}
        <label className="form-field">
          <span className="form-field__label">Ends</span>
          <input
            className="field"
            type="date"
            required
            min={form.startDay}
            value={form.endDay}
            onChange={(e) => {
              set('endDay', e.target.value);
            }}
          />
        </label>
        {!form.allDay && (
          <label className="form-field">
            <span className="form-field__label">
              <span className="visually-hidden">End </span>Time
            </span>
            <input
              className="field"
              type="time"
              required
              value={form.endTime}
              onChange={(e) => {
                set('endTime', e.target.value);
              }}
            />
          </label>
        )}
      </div>
      <label className="form-field">
        <span className="form-field__label">Repeat</span>
        <select
          className="field"
          value={form.repeat}
          onChange={(e) => {
            set('repeat', e.target.value as Form['repeat']);
          }}
        >
          {form.customRule && <option value="keep">{repeatText('keep', start)}</option>}
          {(['none', 'daily', 'weekly', 'monthly', 'yearly'] as const).map((r) => (
            <option key={r} value={r}>
              {repeatText(r, start)}
            </option>
          ))}
        </select>
      </label>
      <label className="form-field">
        <span className="form-field__label">Reminder</span>
        <select
          className="field"
          value={String(form.reminder)}
          onChange={(e) => {
            set('reminder', e.target.value === 'keep' ? 'keep' : Number(e.target.value));
          }}
        >
          {form.reminder === 'keep' && <option value="keep">As set elsewhere</option>}
          {REMINDERS.map((m) => (
            <option key={m} value={m}>
              {reminderText(m)}
            </option>
          ))}
        </select>
      </label>
      <label className="form-field">
        <span className="form-field__label">Location</span>
        <input
          className="field"
          value={form.location}
          maxLength={500}
          onChange={(e) => {
            set('location', e.target.value);
          }}
        />
      </label>
      <label className="form-field">
        <span className="form-field__label">Notes</span>
        <textarea
          id={`${id}-notes`}
          className="field event-editor__notes"
          rows={4}
          value={form.notes}
          onChange={(e) => {
            set('notes', e.target.value);
          }}
        />
      </label>
      <div className="event-editor__actions">
        <button type="button" className="btn btn--secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {busy ? 'Saving…' : 'Save'}
        </button>
      </div>
    </form>
  );
}
