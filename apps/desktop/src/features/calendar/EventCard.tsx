import type { CalendarEvent } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { cssColor, dayText, reminderText, repeatText, whenText } from './calendarTime';
import { ruleText } from './repeatRule';

const STATUS: Record<string, string> = {
  accepted: 'Accepted',
  declined: 'Declined',
  tentative: 'Maybe',
  'needs-action': 'No answer yet',
};

const SHOWN_AS: Record<string, string> = {
  free: 'Shows as free',
  tentative: 'Shows as tentative',
  away: 'Shows as away',
};

/** An event's details, with Edit and Delete (or, read-only, just your reminders). */
export function EventCard({
  event,
  calendarName,
  busy,
  readOnly,
  canDelete,
  onBack,
  onEdit,
  onEditDate,
  onDelete,
  onWrite,
}: {
  event: CalendarEvent;
  calendarName: string;
  busy: boolean;
  /** Why it can't be changed, or null. */
  readOnly: string | null;
  /** Writable calendar: it can be taken out, even when someone else organises it. */
  canDelete: boolean;
  onBack: () => void;
  onEdit: () => void;
  /** One date of a series: change only that date. */
  onEditDate?: (() => void) | undefined;
  onDelete: () => void;
  onWrite: (to: string) => void;
}) {
  const people = [...(event.organizer ? [event.organizer] : []), ...event.attendees].filter(
    (p, i, all) => all.findIndex((q) => q.email.toLowerCase() === p.email.toLowerCase()) === i,
  );
  const start = new Date(event.start * 1000);
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
          {event.repeatRule ? ruleText(event.repeatRule, start) : repeatText(event.repeat, start)}
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
        {SHOWN_AS[event.showAs] ? ` · ${SHOWN_AS[event.showAs] ?? ''}` : ''}
        {event.private ? ' · Private' : ''}
      </p>
      {readOnly && <p className="form-hint">{readOnly}</p>}
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
          {readOnly ? 'Reminders' : 'Edit'}
        </button>
        {onEditDate && !readOnly && (
          <button
            type="button"
            className="btn btn--secondary"
            disabled={busy}
            aria-label={`Edit only ${dayText(start)}`}
            onClick={onEditDate}
          >
            Edit this date
          </button>
        )}
        {canDelete && (
          <button type="button" className="btn btn--danger" disabled={busy} onClick={onDelete}>
            Delete
          </button>
        )}
      </div>
    </article>
  );
}
