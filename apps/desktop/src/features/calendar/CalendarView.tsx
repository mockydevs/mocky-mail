import { Fragment, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, CalendarEvent, CalendarInfo, CalendarStatus, EventInput } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { MenuButton, MenuGroup, MenuItem } from '../../ui/Menu';
import { Modal } from '../../ui/Modal';
import { ConnectCards } from '../connect/ConnectCard';
import { useFileTasks } from '../files/FileTasks';
import { exportCalendarTask, importCalendarTask } from '../files/tasks';
import { CalendarList, MiniMonth } from './CalendarSide';
import {
  addDays,
  addMonths,
  byDay,
  cssColor,
  dayKey,
  dayText,
  eventDays,
  fromDayKey,
  monthGrid,
  seconds,
  startOfDay,
  timeText,
  viewDays,
  weekdayNames,
  type DayKey,
} from './calendarTime';
import { EventCard } from './EventCard';
import { EventEditor } from './EventEditor';
import { copiedInput, formFrom, movedInput, newForm, toInput, type Form } from './eventForm';
import { TimeGrid, type GridColumn, type SlotRange } from './TimeGrid';

interface Props {
  accounts: readonly Account[];
  /** Phone layout: day, month and agenda; details or the editor fill the screen. */
  phone: boolean;
  onClose: () => void;
  /** Start a message to these people. */
  onWrite: (to: string) => void;
  /** Reminder for new events, minutes before; -1 for none. */
  defaultReminder: number;
  /** Today, for tests. */
  today?: Date;
  /** Open this event (from a reminder): its id and when that date starts. */
  focus?: { eventId: string; at: number } | null;
}

type Mode = 'day' | 'workweek' | 'week' | 'month' | 'agenda';

const MODE_LABEL: Record<Mode, string> = {
  day: 'Day',
  workweek: 'Work week',
  week: 'Week',
  month: 'Month',
  agenda: 'Agenda',
};
const COMPUTER_MODES: Mode[] = ['day', 'workweek', 'week', 'month', 'agenda'];
const PHONE_MODES: Mode[] = ['day', 'month', 'agenda'];

type Pane =
  | { kind: 'day' }
  | { kind: 'view'; event: CalendarEvent }
  | { kind: 'edit'; form: Form; series: boolean; event?: CalendarEvent; readOnly: string | null };

/** A question before saving or deleting. */
type Ask =
  | { kind: 'scope'; event: CalendarEvent; run: (scope: 'this' | 'all') => void }
  | { kind: 'notify'; title: string; description: string; yes: string; no: string; run: (notify: boolean) => void };

const MONTH = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' });
const MONTH_ONLY = new Intl.DateTimeFormat(undefined, { month: 'long' });
const DAY_TITLE = new Intl.DateTimeFormat(undefined, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });

/** "November 2026", or "November – December 2026" for a week across two months. */
function rangeTitle(days: readonly Date[]): string {
  const first = days[0] ?? new Date();
  const last = days[days.length - 1] ?? first;
  if (first.getMonth() === last.getMonth()) return MONTH.format(first);
  if (first.getFullYear() === last.getFullYear()) return `${MONTH_ONLY.format(first)} – ${MONTH.format(last)}`;
  return `${MONTH.format(first)} – ${MONTH.format(last)}`;
}

const typing = (target: EventTarget) =>
  target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));

/**
 * The calendar: each account's calendars (CalDAV, Google or Microsoft),
 * kept on the device so they show offline. Day, work week, week, month and
 * agenda views; calendars shown or hidden from a list beside them; add,
 * edit, drag and delete events (one date of a series, or all); invite
 * people and find a time that suits them.
 */
export function CalendarView({ accounts, phone, onClose, onWrite, defaultReminder, today, focus }: Props) {
  const api = useMailApi();
  const files = useFileTasks();
  const id = useId();
  const [clock, setClock] = useState(() => today ?? new Date());
  const now = today ?? clock;
  const [mode, setMode] = useState<Mode>(phone ? 'agenda' : 'month');
  const [month, setMonth] = useState(() => new Date(now.getFullYear(), now.getMonth(), 1));
  const [selected, setSelected] = useState<Date>(() => startOfDay(now));
  const [calendars, setCalendars] = useState<CalendarInfo[]>([]);
  const [statuses, setStatuses] = useState<CalendarStatus[]>([]);
  const [events, setEvents] = useState<CalendarEvent[] | null>(null);
  const [revision, setRevision] = useState(0);
  const [pane, setPane] = useState<Pane>({ kind: 'day' });
  const [error, setError] = useState<AppError | null>(null);
  // Why the events could not be loaded; shown where the events would be,
  // which every layout has on screen (a phone hides the side pane).
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncing, setSyncing] = useState(true);
  const [deleting, setDeleting] = useState<CalendarEvent | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [sideBySide, setSideBySide] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const [pendingFocus, setPendingFocus] = useState<{ eventId: string; at: number } | null>(null);

  // The now-line and "today" move on by themselves.
  useEffect(() => {
    if (today) return;
    const timer = window.setInterval(() => {
      setClock(new Date());
    }, 60_000);
    return () => {
      window.clearInterval(timer);
    };
  }, [today]);

  // Escape (and the phone's Back) closes an event or the editor first.
  const open = useRef(false);
  useEffect(() => {
    open.current = pane.kind !== 'day' && deleting === null;
  });
  useEffect(() => {
    // Heard on the way down from the document, wherever focus is (the
    // sidebar, the page itself), and before the app's own Esc, which would
    // close the whole calendar and with it the event being edited.
    const onKey = (ev: globalThis.KeyboardEvent) => {
      if (ev.key !== 'Escape' || !open.current) return;
      // Another dialog on top answers Esc itself.
      if (document.querySelector('[aria-modal="true"]')) return;
      ev.preventDefault();
      ev.stopPropagation();
      setError(null);
      setPane({ kind: 'day' });
    };
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('keydown', onKey, true);
    };
  }, []);

  // A reminder's "Open": go to that date, then open the event once it's loaded.
  const focusKey = focus ? `${focus.eventId}:${String(focus.at)}` : '';
  const [seenFocus, setSeenFocus] = useState('');
  if (focus && focusKey !== seenFocus) {
    setSeenFocus(focusKey);
    const day = startOfDay(new Date(focus.at * 1000));
    setSelected(day);
    setMonth(new Date(day.getFullYear(), day.getMonth(), 1));
    if (mode === 'agenda') setMode(phone ? 'day' : 'month');
    setPendingFocus(focus);
  }

  const gridMode = mode === 'day' || mode === 'workweek' || mode === 'week';
  const selectedKey = dayKey(selected);
  const days = useMemo(
    () => (mode === 'day' || mode === 'workweek' || mode === 'week' ? viewDays(mode, fromDayKey(selectedKey)) : []),
    [mode, selectedKey],
  );
  const grid = useMemo(() => monthGrid(month), [month]);
  const first = grid[0] ?? month;
  const last = grid[grid.length - 1] ?? month;
  const from = seconds(gridMode ? (days[0] ?? selected) : mode === 'month' ? first : month);
  const to = seconds(
    gridMode ? addDays(days[days.length - 1] ?? selected, 1) : mode === 'month' ? addDays(last, 1) : addMonths(month, 1),
  );

  useEffect(() => {
    let live = true;
    api.listEvents(from, to).then(
      (list) => {
        if (!live) return;
        setEvents(list);
        setLoadError(null);
      },
      (e: unknown) => {
        if (live) setLoadError(toAppError(e));
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
  }, [api, from, to, revision]);

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

  const known = calendars.filter((c) => accounts.some((a) => a.id === c.accountId));
  const hiddenKey = known
    .filter((c) => !c.visible)
    .map((c) => c.id)
    .join(' ');
  const hidden = new Set(hiddenKey.split(' '));
  const shown = (events ?? []).filter((e) => !hidden.has(e.calendarId));
  const byDays = byDay(shown);
  const calendarOf = (calendarId: string) => calendars.find((c) => c.id === calendarId);
  const calendarName = (calendarId: string) => calendarOf(calendarId)?.name ?? 'Calendar';
  const writable = known.filter((c) => c.writable);
  const defaultCalendar = writable.find((c) => c.visible) ?? writable[0];
  /** A calendar's name, with its account's address when that tells two apart. */
  const calendarTitle = (c: CalendarInfo) => {
    const email = accounts.find((a) => a.id === c.accountId)?.email;
    return email ? `${c.name} (${email})` : c.name;
  };
  const ownEmails = accounts.map((a) => a.email);

  // Once the reminder's event is loaded, open it.
  const focused =
    pendingFocus && events
      ? (events.find((e) => e.id === pendingFocus.eventId && e.start === pendingFocus.at) ??
        events.find((e) => e.id === pendingFocus.eventId))
      : undefined;
  if (focused) {
    setPendingFocus(null);
    setPane({ kind: 'view', event: focused });
  }

  /** Why an event can't be changed here, or null. */
  const readOnlyReason = (e: CalendarEvent): string | null => {
    const cal = calendarOf(e.calendarId);
    if (cal && !cal.writable) return `${cal.name} is read-only. You can change your own reminders.`;
    if (!e.isOrganizer) {
      const who = e.organizer ? (e.organizer.name ?? e.organizer.email) : 'Someone else';
      return `${who} organises this event, so only they can change it. You can change your own reminders; answer from the invitation email.`;
    }
    return null;
  };
  const canEdit = (e: CalendarEvent) => readOnlyReason(e) === null;

  const startNew = (day: Date, startMins?: number, lengthMins?: number, allDay = false, calendarId?: string | null, title = '') => {
    const target = (calendarId ? writable.find((c) => c.id === calendarId) : undefined) ?? defaultCalendar;
    if (!target) return;
    setError(null);
    const form = newForm(day, target.id, now, defaultReminder, startMins, lengthMins, allDay);
    setPane({ kind: 'edit', form: { ...form, title }, series: false, readOnly: null });
  };

  const openEditor = (e: CalendarEvent, onlyThisDate = false) => {
    setError(null);
    const readOnly = readOnlyReason(e);
    if (onlyThisDate && e.recurrenceId !== null) {
      setPane({ kind: 'edit', form: formFrom(e, ownEmails, e.recurrenceId), series: false, event: e, readOnly });
      return;
    }
    if (!e.recurring) {
      setPane({ kind: 'edit', form: formFrom(e, ownEmails), series: false, event: e, readOnly });
      return;
    }
    // A series is edited from its first date.
    api.getEvent(e.id).then(
      (master) => {
        setPane({ kind: 'edit', form: formFrom(master, ownEmails), series: true, event: master, readOnly });
      },
      (err: unknown) => {
        setError(toAppError(err));
      },
    );
  };

  /** Save, then show the event (`show`) or stay where the user is. */
  const commit = (input: EventInput, show: boolean) => {
    setBusy(true);
    setError(null);
    api.saveEvent(input).then(
      (saved) => {
        setBusy(false);
        if (show) {
          setSelected(eventDays(saved)[0]);
          setPane({ kind: 'view', event: saved });
        }
        setRevision((r) => r + 1);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  /**
   * Whether to email the people invited: Microsoft sends invitations and
   * updates itself, so it isn't asked there.
   */
  const withNotify = (
    input: EventInput,
    people: number,
    isNew: boolean,
    then: (input: EventInput) => void,
  ) => {
    const provider = calendarOf(input.calendarId)?.provider;
    if (people === 0) {
      then(input);
      return;
    }
    if (provider === 'microsoft') {
      then({ ...input, notifyAttendees: true });
      return;
    }
    const n = people === 1 ? 'the person' : `the ${String(people)} people`;
    setAsk({
      kind: 'notify',
      title: isNew ? 'Send invitations?' : 'Send updates?',
      description: isNew
        ? `Email an invitation to ${n} you invited?`
        : `Let ${n} invited know about the change?`,
      yes: 'Send',
      no: "Don't send",
      run: (notify) => {
        then({ ...input, notifyAttendees: notify });
      },
    });
  };

  const save = (form: Form, current: Extract<Pane, { kind: 'edit' }>) => {
    const editable = current.readOnly === null;
    const input = toInput(form, editable);
    if (typeof input === 'string') {
      setError({ kind: 'invalid', message: input });
      return;
    }
    const invited = input.attendees ?? [];
    const added = invited.some((a) => !form.invited.includes(a.toLowerCase()));
    const removed = form.invited.some((a) => !invited.some((b) => b.toLowerCase() === a));
    const people = editable ? Math.max(invited.length, removed ? form.invited.length : 0) : 0;
    withNotify(input, people, !form.id || added, (next) => {
      commit(next, true);
    });
  };

  /** An event was dragged or resized in the time grid. */
  const moveEvent = (e: CalendarEvent, start: number, end: number, copy: boolean) => {
    if (copy) {
      commit(copiedInput(e, start, end), false);
      return;
    }
    const people = e.isOrganizer ? e.attendees.filter((p) => !ownEmails.includes(p.email)).length : 0;
    const go = (input: EventInput) => {
      withNotify(input, people, false, (next) => {
        commit(next, false);
      });
    };
    if (e.recurring && e.recurrenceId !== null) {
      const occurrence = e.recurrenceId;
      setAsk({
        kind: 'scope',
        event: e,
        run: (scope) => {
          if (scope === 'this') {
            go({ ...movedInput(e, start, end), occurrence, scope: 'this' });
            return;
          }
          api.getEvent(e.id).then(
            (master) => {
              go({ ...movedInput(master, master.start + (start - e.start), master.end + (end - e.end)), scope: 'all' });
            },
            (err: unknown) => {
              setError(toAppError(err));
            },
          );
        },
      });
      return;
    }
    go(movedInput(e, start, end));
  };

  const remove = (e: CalendarEvent, onlyThisDate: boolean, notify: boolean) => {
    setDeleting(null);
    setBusy(true);
    // Without a date to remove, "only this date" must never reach the
    // backend as null, which means the whole series.
    if (onlyThisDate && e.recurrenceId === null) {
      setBusy(false);
      setError({ kind: 'invalid', message: 'Open the date you want to remove from the calendar, then delete it there.' });
      return;
    }
    const done = () => {
      setBusy(false);
      setPane({ kind: 'day' });
      setRevision((r) => r + 1);
    };
    const failed = (err: unknown) => {
      setBusy(false);
      setError(toAppError(err));
    };
    if (notify) api.deleteEvent(e.id, onlyThisDate ? e.recurrenceId : null, true).then(done, failed);
    else api.deleteEvent(e.id, onlyThisDate ? e.recurrenceId : null).then(done, failed);
  };

  /** Delete, asking which dates of a series, and whether to tell the people invited. */
  const removeAsking = (e: CalendarEvent, onlyThisDate: boolean) => {
    const people = e.isOrganizer ? e.attendees.filter((p) => !ownEmails.includes(p.email)).length : 0;
    if (people === 0) {
      remove(e, onlyThisDate, false);
      return;
    }
    if (calendarOf(e.calendarId)?.provider === 'microsoft') {
      remove(e, onlyThisDate, true);
      return;
    }
    setDeleting(null);
    setAsk({
      kind: 'notify',
      title: 'Send cancellations?',
      description: `Tell ${people === 1 ? 'the person' : `the ${String(people)} people`} invited that it's cancelled?`,
      yes: 'Send',
      no: "Don't send",
      run: (notify) => {
        remove(e, onlyThisDate, notify);
      },
    });
  };

  const requestDelete = (e: CalendarEvent) => {
    const cal = calendarOf(e.calendarId);
    if (cal && !cal.writable) {
      setError({ kind: 'invalid', message: `${cal.name} is read-only.` });
      return;
    }
    if (e.recurring) setDeleting(e);
    else removeAsking(e, false);
  };

  const toggleCalendar = (c: CalendarInfo, visible: boolean) => {
    setCalendars((list) => list.map((x) => (x.id === c.id ? { ...x, visible } : x)));
    api.setCalendarVisible(c.id, visible).then(setCalendars, (e: unknown) => {
      setError(toAppError(e));
      setCalendars((list) => list.map((x) => (x.id === c.id ? { ...x, visible: !visible } : x)));
    });
  };

  const step = (n: number) => {
    setPane({ kind: 'day' });
    if (mode === 'month' || mode === 'agenda') {
      setMonth((m) => addMonths(m, n));
      return;
    }
    const next = addDays(selected, n * (mode === 'day' ? 1 : 7));
    setSelected(next);
    setMonth(new Date(next.getFullYear(), next.getMonth(), 1));
  };

  const pick = (day: Date) => {
    setSelected(startOfDay(day));
    setMonth(new Date(day.getFullYear(), day.getMonth(), 1));
    setPane({ kind: 'day' });
    setDrawer(false);
  };

  const unit = mode === 'month' || mode === 'agenda' ? 'month' : mode === 'day' ? 'day' : 'week';
  const todayKey = dayKey(now);
  const notes = statuses.filter((s) => s.state !== 'synced' && !s.connectable);
  const showMain = !phone || pane.kind === 'day';
  const showSide = !phone || pane.kind !== 'day';
  const modes = phone ? PHONE_MODES : COMPUTER_MODES;
  const title = gridMode
    ? mode === 'day'
      ? phone
        ? dayText(selected)
        : DAY_TITLE.format(selected)
      : rangeTitle(days)
    : MONTH.format(month);
  const visibleCalendars = known.filter((c) => c.visible);
  const splitDay = mode === 'day' && sideBySide && !phone && visibleCalendars.length > 1;
  const columns: GridColumn[] = splitDay
    ? visibleCalendars.map((c) => ({ key: c.id, day: selected, calendarId: c.id, label: c.name, color: c.color }))
    : days.map((d) => ({ key: dayKey(d), day: d, calendarId: null }));

  const eventButton = (e: CalendarEvent, compact: boolean) => (
    <button
      key={`${e.id}:${String(e.start)}`}
      type="button"
      className={`cal-event${compact ? ' cal-event--chip' : ''}${e.allDay ? ' cal-event--allday' : ''}${e.showAs === 'tentative' ? ' cal-event--tentative' : ''}`}
      style={{ ['--event-color' as string]: cssColor(e.color) }}
      onClick={(ev) => {
        ev.stopPropagation();
        setError(null);
        setSelected(eventDays(e)[0]);
        setPane({ kind: 'view', event: e });
      }}
      onKeyDown={(ev) => {
        if (ev.key === 'Delete') {
          ev.preventDefault();
          ev.stopPropagation();
          requestDelete(e);
        }
      }}
    >
      {!e.allDay && <span className="cal-event__time">{timeText(e.start)}</span>}
      <span className="cal-event__title">{e.title || '(No title)'}</span>
      {!compact && e.location && <span className="cal-event__place">{e.location}</span>}
    </button>
  );

  const onSectionKey = (ev: KeyboardEvent<HTMLElement>) => {
    if ((ev.key !== 'n' && ev.key !== 'N') || ev.ctrlKey || ev.metaKey || ev.altKey || typing(ev.target)) return;
    if (pane.kind === 'edit' || document.querySelector('[aria-modal="true"]')) return;
    ev.preventDefault();
    startNew(selected);
  };

  const onGridDay = (day: Date, n: number) => {
    let next = day;
    if (mode === 'workweek') {
      const d = next.getDay();
      if (d === 6) next = addDays(next, n > 0 ? 2 : -1);
      else if (d === 0) next = addDays(next, n > 0 ? 1 : -2);
    }
    setSelected(startOfDay(next));
    setMonth(new Date(next.getFullYear(), next.getMonth(), 1));
  };

  const side = (
    <>
      <MiniMonth selected={selected} todayKey={todayKey} onPick={pick} />
      <CalendarList accounts={accounts} calendars={known} onToggle={toggleCalendar} />
    </>
  );

  return (
    // The section hears N for a new event; the keys themselves belong to its controls.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <section className="calendar" aria-labelledby={`${id}-title`} onKeyDown={onSectionKey}>
      <header className="calendar__head">
        <h1 id={`${id}-title`} className="calendar__title">
          {title}
        </h1>
        <div className="calendar__nav">
          <button
            type="button"
            className="icon-btn"
            aria-label={`Previous ${unit}`}
            onClick={() => {
              step(-1);
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
            aria-label={`Next ${unit}`}
            onClick={() => {
              step(1);
            }}
          >
            <Icon name="right" />
          </button>
        </div>
        <div className="segmented calendar__views" role="group" aria-label="View">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              className={`segmented__item${mode === m ? ' segmented__item--on' : ''}`}
              aria-pressed={mode === m}
              onClick={() => {
                if (m === 'month' || m === 'agenda') setMonth(new Date(selected.getFullYear(), selected.getMonth(), 1));
                setMode(m);
              }}
            >
              {MODE_LABEL[m]}
            </button>
          ))}
        </div>
        {mode === 'day' && !phone && visibleCalendars.length > 1 && (
          <button
            type="button"
            className={`btn btn--secondary btn--small${sideBySide ? ' btn--on' : ''}`}
            aria-pressed={sideBySide}
            title="Show each calendar in a column of its own"
            onClick={() => {
              setSideBySide((on) => !on);
            }}
          >
            Side by side
          </button>
        )}
        {phone && (
          <button
            type="button"
            className="icon-btn calendar__calendars"
            aria-label="Calendars"
            onClick={() => {
              setDrawer(true);
            }}
          >
            <Icon name="menu" />
          </button>
        )}
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
        {known.length > 0 && (
          <MenuButton label="Import and export calendars" icon="more" title="Import and export">
            {known.map((c) => {
              // Events can only be imported into a calendar the account can change.
              const items = (
                <>
                  {c.writable && (
                    <MenuItem
                      icon="upload"
                      onSelect={() => {
                        files.run(
                          importCalendarTask(api, c, () => {
                            setRevision((r) => r + 1);
                          }),
                        );
                      }}
                    >
                      Import .ics…
                    </MenuItem>
                  )}
                  <MenuItem
                    icon="download"
                    onSelect={() => {
                      files.run(exportCalendarTask(api, c));
                    }}
                  >
                    Export calendar…
                  </MenuItem>
                </>
              );
              return known.length > 1 ? (
                <MenuGroup key={c.id} label={calendarTitle(c)}>
                  {items}
                </MenuGroup>
              ) : (
                <Fragment key={c.id}>{items}</Fragment>
              );
            })}
          </MenuButton>
        )}
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
      <ConnectCards
        accounts={accounts}
        statuses={statuses}
        onConnected={() => {
          setRevision((r) => r + 1);
        }}
      />

      <div className="calendar__body">
        {!phone && <aside className="calendar__nav-pane" aria-label="Dates and calendars">{side}</aside>}
        {showMain && (
          <div className={`calendar__main${gridMode ? ' calendar__main--grid' : ''}`}>
            {loadError && (
              <p className="form-error" role="alert">
                Your events could not be loaded. {loadError.message}
              </p>
            )}
            {events === null ? (
              loadError === null && (
                <p className="muted calendar__empty" role="status">
                  Loading events…
                </p>
              )
            ) : gridMode ? (
              <TimeGrid
                columns={columns}
                events={shown}
                now={now}
                selectedKey={selectedKey}
                interactive={!phone}
                canAdd={writable.length > 0}
                canEdit={canEdit}
                label={mode === 'day' ? dayText(selected) : `${MODE_LABEL[mode]} of ${dayText(days[0] ?? selected)}`}
                onOpen={(e) => {
                  setError(null);
                  setPane({ kind: 'view', event: e });
                }}
                onCreate={(r: SlotRange, text) => {
                  startNew(r.column.day, r.allDay ? undefined : r.start, r.end - r.start, r.allDay, r.column.calendarId, text);
                }}
                onQuickSave={(r, text) => {
                  if (!text.trim()) return;
                  const target =
                    (r.column.calendarId ? writable.find((c) => c.id === r.column.calendarId) : undefined) ?? defaultCalendar;
                  if (!target) return;
                  const form = newForm(r.column.day, target.id, now, defaultReminder, r.start, r.end - r.start, r.allDay);
                  const input = toInput({ ...form, title: text.trim() });
                  if (typeof input === 'string') setError({ kind: 'invalid', message: input });
                  else commit(input, false);
                }}
                onChange={moveEvent}
                onDelete={requestDelete}
                onSelectDay={onGridDay}
                onDayHeader={
                  mode === 'day'
                    ? undefined
                    : (d) => {
                        setSelected(startOfDay(d));
                        setMode('day');
                      }
                }
              />
            ) : mode === 'month' ? (
              <MonthGrid
                month={month}
                grid={grid}
                days={byDays}
                selectedKey={selectedKey}
                todayKey={todayKey}
                render={(e) => eventButton(e, true)}
                onSelect={(day) => {
                  setSelected(day);
                  setPane({ kind: 'day' });
                }}
                onMoveTo={(next) => {
                  setSelected(next);
                  if (next.getMonth() !== month.getMonth()) setMonth(new Date(next.getFullYear(), next.getMonth(), 1));
                }}
                onNew={(day) => {
                  startNew(day);
                }}
              />
            ) : (
              <Agenda days={byDays} month={month} todayKey={todayKey} render={(e) => eventButton(e, false)} />
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
                events={byDays.get(selectedKey) ?? []}
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
                readOnly={readOnlyReason(pane.event)}
                canDelete={calendarOf(pane.event.calendarId)?.writable !== false}
                onBack={() => {
                  setPane({ kind: 'day' });
                }}
                onEdit={() => {
                  openEditor(pane.event);
                }}
                onEditDate={
                  pane.event.recurring && pane.event.recurrenceId !== null
                    ? () => {
                        openEditor(pane.event, true);
                      }
                    : undefined
                }
                onDelete={() => {
                  requestDelete(pane.event);
                }}
                onWrite={onWrite}
              />
            )}
            {pane.kind === 'edit' && (
              <EventEditor
                key={`${pane.form.id ?? 'new'}:${String(pane.form.occurrence)}`}
                initial={pane.form}
                series={pane.series}
                calendars={pane.readOnly ? known : writable}
                accounts={accounts}
                readOnly={pane.readOnly}
                event={pane.event}
                busy={busy}
                now={seconds(now)}
                onCancel={() => {
                  setError(null);
                  setPane({ kind: 'day' });
                }}
                onSave={(form) => {
                  save(form, pane);
                }}
              />
            )}
          </aside>
        )}
      </div>

      {phone && drawer && (
        <Modal
          title="Calendars"
          size="large"
          onClose={() => {
            setDrawer(false);
          }}
        >
          <div className="cal-drawer">{side}</div>
          <div className="modal__actions">
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setDrawer(false);
              }}
            >
              Done
            </button>
          </div>
        </Modal>
      )}

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
                  removeAsking(deleting, true);
                }}
              >
                Only this date
              </button>
            )}
            <button
              type="button"
              className="btn btn--danger"
              onClick={() => {
                removeAsking(deleting, false);
              }}
            >
              Every date
            </button>
          </div>
        </Modal>
      )}

      {ask?.kind === 'scope' && (
        <Modal
          title="Just this one or the whole series?"
          description={`“${ask.event.title || '(No title)'}” repeats. Change only ${dayText(eventDays(ask.event)[0])}, or every date?`}
          size="small"
          onClose={() => {
            setAsk(null);
          }}
        >
          <div className="modal__actions">
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setAsk(null);
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setAsk(null);
                ask.run('all');
              }}
            >
              The whole series
            </button>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setAsk(null);
                ask.run('this');
              }}
            >
              Just this one
            </button>
          </div>
        </Modal>
      )}

      {ask?.kind === 'notify' && (
        <Modal
          title={ask.title}
          description={ask.description}
          size="small"
          onClose={() => {
            setAsk(null);
          }}
        >
          <div className="modal__actions">
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setAsk(null);
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setAsk(null);
                ask.run(false);
              }}
            >
              {ask.no}
            </button>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setAsk(null);
                ask.run(true);
              }}
            >
              {ask.yes}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}

function MonthGrid({
  month,
  grid,
  days,
  selectedKey,
  todayKey,
  render,
  onSelect,
  onMoveTo,
  onNew,
}: {
  month: Date;
  grid: Date[];
  days: Map<DayKey, CalendarEvent[]>;
  selectedKey: DayKey;
  todayKey: DayKey;
  render: (e: CalendarEvent) => ReactNode;
  onSelect: (day: Date) => void;
  onMoveTo: (day: Date) => void;
  onNew: (day: Date) => void;
}) {
  return (
    <div className="month" role="grid" aria-label={MONTH.format(month)}>
      <div className="month__row month__row--head" role="row">
        {weekdayNames().map((w) => (
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
            const shownList = list.slice(0, 3);
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
                  onSelect(day);
                }}
                onDoubleClick={() => {
                  onNew(day);
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
                    onMoveTo(addDays(day, move));
                    requestAnimationFrame(() => {
                      document.querySelector<HTMLElement>('.month__day[tabindex="0"]')?.focus();
                    });
                  } else if (ev.key === 'Enter') {
                    onSelect(day);
                  }
                }}
              >
                <span className="month__date">{day.getDate()}</span>
                <span className="month__events">
                  {shownList.map(render)}
                  {list.length > shownList.length && (
                    <span className="month__more">+{list.length - shownList.length} more</span>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      ))}
    </div>
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
