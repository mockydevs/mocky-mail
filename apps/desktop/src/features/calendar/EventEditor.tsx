import { useId, useState, type SyntheticEvent } from 'react';

import type { Account, CalendarEvent, CalendarInfo, ShowAs } from '../../api/types';
import { RecipientField } from '../compose/RecipientField';
import { Icon } from '../../ui/Icon';
import { addDays, dayKey, dayText, deviceZone, fromDayKey, reminderText, repeatText, timeValue } from './calendarTime';
import { attendeesOf, formTimes, REMINDERS, type Form, type RepeatChoice } from './eventForm';
import { FindTime } from './FindTime';
import { RepeatDialog } from './RepeatDialog';
import { ruleText, weekdaysRule } from './repeatRule';

interface Props {
  initial: Form;
  /** Editing a whole series. */
  series: boolean;
  /** Calendars events can go in. */
  calendars: CalendarInfo[];
  accounts: readonly Account[];
  /** Why the event can't be changed (only your reminders can), or null. */
  readOnly: string | null;
  /** The event as stored, for your answer to an invitation. */
  event?: CalendarEvent | undefined;
  busy: boolean;
  /** Unix seconds, for Find a Time. */
  now: number;
  onCancel: () => void;
  onSave: (form: Form) => void;
}

const SHOW_AS: { value: ShowAs; label: string }[] = [
  { value: 'busy', label: 'Busy' },
  { value: 'tentative', label: 'Tentative' },
  { value: 'free', label: 'Free' },
  { value: 'away', label: 'Away' },
];

const ANSWER: Record<string, string> = {
  accepted: 'You accepted',
  declined: 'You declined',
  tentative: 'You said maybe',
  'needs-action': "You haven't answered yet",
};

/**
 * The event editor: when and where, how it repeats (presets or a custom
 * rule), reminders, how it shows you, and the people invited, with Find a
 * Time. A read-only event shows the same fields, and only reminders change.
 */
export function EventEditor({ initial, series, calendars, accounts, readOnly, event, busy, now, onCancel, onSave }: Props) {
  const id = useId();
  const [form, setForm] = useState(initial);
  const [repeating, setRepeating] = useState(false);
  const [finding, setFinding] = useState(false);
  const set = <K extends keyof Form>(key: K, value: Form[K]) => {
    setForm((f) => ({ ...f, [key]: value }));
  };
  const start = fromDayKey(form.startDay);
  const locked = readOnly !== null;
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
  const calendar = calendars.find((c) => c.id === form.calendarId);
  const account = accounts.find((a) => a.id === (calendar?.accountId ?? event?.accountId));
  const ownEmail = account?.email ?? '';
  const myAnswer = event?.attendees.find((p) => p.email.toLowerCase() === ownEmail.toLowerCase())?.status ?? null;
  const people = attendeesOf(form).emails;
  const title = locked ? 'Event' : form.id ? 'Edit event' : 'New event';

  const repeatValue: RepeatChoice | 'custom' = form.repeat;
  const chooseRepeat = (value: string) => {
    if (value === 'custom') {
      setRepeating(true);
      return;
    }
    setForm((f) => ({ ...f, repeat: value as RepeatChoice }));
  };

  const setReminder = (index: number, value: number) => {
    setForm((f) => {
      const next = [...f.reminders];
      if (value < 0) next.splice(index, 1);
      else if (index >= next.length) next.push(value);
      else next[index] = value;
      return { ...f, reminders: [...new Set(next)].sort((a, b) => a - b) };
    });
  };
  const reminderRows = form.reminders.length === 0 ? [-1] : form.reminders;
  const unused = REMINDERS.find((m) => m >= 0 && !form.reminders.includes(m) && m > (form.reminders.at(-1) ?? -1));

  return (
    <>
      <form
        className={`event-editor${locked ? ' event-editor--locked' : ''}`}
        aria-label={form.id ? 'Edit event' : 'New event'}
        noValidate
        onSubmit={submit}
      >
        <h2 className="event-editor__title">{title}</h2>
        {readOnly && <p className="form-hint">{readOnly}</p>}
        {series && !locked && <p className="form-hint">This event repeats. Changes apply to every date.</p>}
        {form.occurrence !== null && !locked && (
          <p className="form-hint">Changes apply to {dayText(new Date(form.occurrence * 1000))} only.</p>
        )}
        {locked && event?.organizer && (
          <p className="event-editor__answer">
            <Icon name="people" size={16} />
            <span>
              {ANSWER[myAnswer ?? 'needs-action'] ?? ANSWER['needs-action']}
              {` · organised by ${event.organizer.name ?? event.organizer.email}`}
            </span>
          </p>
        )}
        <label className="form-field">
          <span className="form-field__label">Title</span>
          <input
            className="field"
            value={form.title}
            maxLength={500}
            readOnly={locked}
            // The first field of a form the user just opened.
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus={!locked}
            onChange={(e) => {
              set('title', e.target.value);
            }}
          />
        </label>
        {calendars.length > 1 && !locked && form.occurrence === null && (
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
            disabled={locked}
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
              readOnly={locked}
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
                readOnly={locked}
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
              readOnly={locked}
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
                readOnly={locked}
                value={form.endTime}
                onChange={(e) => {
                  set('endTime', e.target.value);
                }}
              />
            </label>
          )}
        </div>
        {!form.allDay && <p className="form-hint event-editor__zone">Times are in {deviceZone().replace(/_/g, ' ')}</p>}
        {form.occurrence === null && (
          <label className="form-field">
            <span className="form-field__label">Repeat</span>
            <select
              className="field"
              value={repeatValue}
              disabled={locked}
              onChange={(e) => {
                chooseRepeat(e.target.value);
              }}
            >
              {form.repeat === 'keep' && <option value="keep">{repeatText('keep', start)}</option>}
              {form.repeatRule && <option value="rule">{ruleText(form.repeatRule, start)}</option>}
              {(['none', 'daily', 'weekly'] as const).map((r) => (
                <option key={r} value={r}>
                  {repeatText(r, start)}
                </option>
              ))}
              <option value="weekdays">{ruleText(weekdaysRule(), start)}</option>
              {(['monthly', 'yearly'] as const).map((r) => (
                <option key={r} value={r}>
                  {repeatText(r, start)}
                </option>
              ))}
              <option value="custom">Custom…</option>
            </select>
          </label>
        )}
        <fieldset className="event-editor__reminders">
          <legend className="visually-hidden">Reminders</legend>
          {reminderRows.map((value, index) => (
            <label key={`${String(index)}-${String(value)}`} className="form-field">
              <span className="form-field__label">{index === 0 ? 'Reminder' : `Reminder ${String(index + 1)}`}</span>
              <select
                className="field"
                value={String(value)}
                onChange={(e) => {
                  setReminder(index, Number(e.target.value));
                }}
              >
                {[...new Set([...REMINDERS, value])]
                  .filter((m) => m === value || m < 0 || !form.reminders.includes(m))
                  .sort((a, b) => a - b)
                  .map((m) => (
                    <option key={m} value={m}>
                      {reminderText(m)}
                    </option>
                  ))}
              </select>
            </label>
          ))}
          {form.reminders.length > 0 && form.reminders.length < 5 && unused !== undefined && (
            <button
              type="button"
              className="btn btn--link"
              onClick={() => {
                setReminder(form.reminders.length, unused);
              }}
            >
              <Icon name="plus" size={14} />
              Add another reminder
            </button>
          )}
        </fieldset>
        <div className="event-editor__row">
          <label className="form-field">
            <span className="form-field__label">Show as</span>
            <select
              className="field"
              value={form.showAs}
              disabled={locked}
              onChange={(e) => {
                set('showAs', e.target.value as ShowAs);
              }}
            >
              {SHOW_AS.map((s) => (
                <option key={s.value} value={s.value}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
          <label className="choice event-editor__private">
            <input
              type="checkbox"
              checked={form.private}
              disabled={locked}
              onChange={(e) => {
                set('private', e.target.checked);
              }}
            />
            Private
          </label>
        </div>
        {(!locked || people.length > 0) && (
          <div className="form-field event-editor__people">
            <label className="form-field__label" htmlFor={`${id}-people`}>
              Invite people
            </label>
            {locked ? (
              <p id={`${id}-people`}>{people.join(', ')}</p>
            ) : (
              <div className="event-editor__recipients">
                <RecipientField
                  id={`${id}-people`}
                  label="Invited"
                  value={form.attendees}
                  onChange={(value) => {
                    set('attendees', value);
                  }}
                  invalid={attendeesOf(form).invalid.length > 0}
                  describedBy={undefined}
                  // Not focused: the title is.
                  // eslint-disable-next-line jsx-a11y/no-autofocus
                  autoFocus={false}
                />
              </div>
            )}
            {!locked && calendar?.provider === 'microsoft' && people.length > 0 && (
              <p className="form-hint">Microsoft emails invitations and updates to the people invited by itself.</p>
            )}
            {!locked && !form.allDay && (
              <button
                type="button"
                className="btn btn--secondary btn--small event-editor__find"
                disabled={!account}
                onClick={() => {
                  setFinding(true);
                }}
              >
                <Icon name="clock" size={16} />
                Find a time
              </button>
            )}
          </div>
        )}
        <label className="form-field">
          <span className="form-field__label">Location</span>
          <input
            className="field"
            value={form.location}
            maxLength={500}
            readOnly={locked}
            onChange={(e) => {
              set('location', e.target.value);
            }}
          />
        </label>
        <label className="form-field">
          <span className="form-field__label">Notes</span>
          <textarea
            className="field event-editor__notes"
            rows={4}
            readOnly={locked}
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
            {busy ? 'Saving…' : locked ? 'Save reminders' : 'Save'}
          </button>
        </div>
      </form>
      {repeating && (
        <RepeatDialog
          start={start}
          initial={form.repeat === 'rule' ? form.repeatRule : null}
          onClose={() => {
            setRepeating(false);
          }}
          onDone={(rule) => {
            setRepeating(false);
            setForm((f) => ({ ...f, repeat: 'rule', repeatRule: rule }));
          }}
        />
      )}
      {finding && account && (
        <FindTime
          accountId={account.id}
          ownEmail={account.email}
          ownCalendarIds={calendars.filter((c) => c.accountId === account.id).map((c) => c.id)}
          attendees={people}
          start={formTimes(form).start}
          duration={Math.max(1800, formTimes(form).end - formTimes(form).start)}
          eventId={form.id}
          now={now}
          onClose={() => {
            setFinding(false);
          }}
          onPick={(s, e) => {
            setFinding(false);
            const a = new Date(s * 1000);
            const b = new Date(e * 1000);
            setForm((f) => ({
              ...f,
              startDay: dayKey(a),
              startTime: timeValue(a),
              endDay: dayKey(b),
              endTime: timeValue(b),
            }));
          }}
        />
      )}
    </>
  );
}
