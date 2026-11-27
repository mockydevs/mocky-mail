import { useEffect, useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, DueReminder } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { MenuButton, MenuItem } from '../../ui/Menu';
import { dayText, timeText } from '../calendar/calendarTime';

interface Props {
  /** Phones: a full-screen sheet instead of a floating panel. */
  phone: boolean;
  /** Unix seconds. */
  now: () => number;
  /** Open what the reminder is about. */
  onOpen: (reminder: DueReminder) => void;
}

const KIND_ICON: Record<DueReminder['kind'], IconName> = { event: 'calendar', task: 'check', flag: 'flag' };
const KIND_LABEL: Record<DueReminder['kind'], string> = { event: 'Event', task: 'Task', flag: 'Flagged message' };

/** Snooze choices, minutes (`tomorrow`: 09:00 tomorrow). */
const SNOOZE: { label: string; minutes: number | 'tomorrow' }[] = [
  { label: '5 minutes', minutes: 5 },
  { label: '10 minutes', minutes: 10 },
  { label: '15 minutes', minutes: 15 },
  { label: '30 minutes', minutes: 30 },
  { label: '1 hour', minutes: 60 },
  { label: 'Tomorrow', minutes: 'tomorrow' },
];

/** Minutes from now until 09:00 tomorrow. */
export function minutesUntilTomorrow(now: number): number {
  const d = new Date(now * 1000);
  const morning = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1, 9, 0);
  return Math.max(1, Math.round((morning.getTime() / 1000 - now) / 60));
}

/** "Starts in 15 minutes", "Started 5 minutes ago", "Due today". */
export function whenDue(r: DueReminder, now: number): string {
  if (r.kind !== 'event') {
    const due = new Date(r.at * 1000);
    const today = new Date(now * 1000);
    const same = due.toDateString() === today.toDateString();
    return same ? `Due today, ${timeText(r.at)}` : `Due ${dayText(due)}`;
  }
  const mins = Math.round((r.at - now) / 60);
  const span = (m: number) => (m >= 120 ? `${String(Math.round(m / 60))} hours` : m === 1 ? '1 minute' : `${String(m)} minutes`);
  if (mins > 0 && mins < 24 * 60) return `Starts in ${span(mins)} (${timeText(r.at)})`;
  if (mins >= 24 * 60) return `Starts ${dayText(new Date(r.at * 1000))}, ${timeText(r.at)}`;
  if (mins === 0) return `Starting now (${timeText(r.at)})`;
  return `Started ${span(-mins)} ago (${timeText(r.at)})`;
}

/**
 * Reminders that are due (events, tasks and flagged messages), from the
 * backend's reminder clock. A floating panel on computers, a sheet on phones:
 * dismiss, snooze or open each one.
 */
export function RemindersPanel({ phone, now, onOpen }: Props) {
  const api = useMailApi();
  const id = useId();
  const [reminders, setReminders] = useState<DueReminder[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  /** Closed with the list as it was: it comes back when something new is due. */
  const [closedAt, setClosedAt] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    const off = api.on('mail://reminders', ({ reminders: next }) => {
      setReminders(next);
    });
    api.listDueReminders().then(
      (list) => {
        if (live) setReminders(list);
      },
      () => undefined,
    );
    return () => {
      live = false;
      off();
    };
  }, [api]);

  const keys = reminders.map((r) => r.key);
  const closed = closedAt !== null && keys.every((k) => closedAt.split('\n').includes(k));
  if (reminders.length === 0 || closed) return null;

  const run = (call: Promise<DueReminder[]>) => {
    setError(null);
    call.then(setReminders, (e: unknown) => {
      setError(toAppError(e));
    });
  };
  const dismissAll = () => {
    setError(null);
    const all = [...reminders];
    all
      .reduce<Promise<DueReminder[]>>((chain, r) => chain.then(() => api.dismissReminder(r.key)), Promise.resolve([]))
      .then(setReminders, (e: unknown) => {
        setError(toAppError(e));
      });
  };
  const t = now();

  return (
    <section
      className={`reminders${phone ? ' reminders--sheet' : ''}`}
      role="dialog"
      aria-modal={phone ? 'true' : 'false'}
      aria-labelledby={`${id}-title`}
    >
      <header className="reminders__head">
        <Icon name="bell" />
        <h2 id={`${id}-title`} className="reminders__title">
          {reminders.length === 1 ? 'Reminder' : `${String(reminders.length)} reminders`}
        </h2>
        <button type="button" className="btn btn--link btn--small" onClick={dismissAll}>
          Dismiss all
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label="Close reminders"
          onClick={() => {
            setClosedAt(keys.join('\n'));
          }}
        >
          <Icon name="close" />
        </button>
      </header>
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <ul className="reminders__list" aria-label="Due reminders">
        {reminders.map((r) => (
          <li key={r.key} className="reminders__item" aria-label={`${KIND_LABEL[r.kind]}: ${r.title}`}>
            <span className={`reminders__icon reminders__icon--${r.kind}`} aria-hidden="true">
              <Icon name={KIND_ICON[r.kind]} />
            </span>
            <div className="reminders__text">
              <p className="reminders__what">{r.title}</p>
              <p className="muted small">
                {whenDue(r, t)}
                {r.location ? ` · ${r.location}` : ''}
              </p>
            </div>
            <div className="reminders__actions">
              <button
                type="button"
                className="btn btn--secondary btn--small"
                aria-label={`Open ${r.title}`}
                onClick={() => {
                  onOpen(r);
                }}
              >
                Open
              </button>
              <MenuButton label={`Snooze ${r.title}`} text="Snooze" icon="clock" className="btn btn--secondary btn--small" menuLabel="Snooze for">
                {SNOOZE.map((s) => (
                  <MenuItem
                    key={s.label}
                    onSelect={() => {
                      run(api.snoozeReminder(r.key, s.minutes === 'tomorrow' ? minutesUntilTomorrow(now()) : s.minutes));
                    }}
                  >
                    {s.label}
                  </MenuItem>
                ))}
              </MenuButton>
              <button
                type="button"
                className="btn btn--secondary btn--small"
                aria-label={`Dismiss ${r.title}`}
                onClick={() => {
                  run(api.dismissReminder(r.key));
                }}
              >
                Dismiss
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
