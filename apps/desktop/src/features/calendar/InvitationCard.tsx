import { useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Answer, AppError, CalendarEvent, InvitationView } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { repeatText, whenText } from './calendarTime';

interface Props {
  messageId: string;
  initial: InvitationView;
}

const ANSWERS: { answer: Answer; label: string }[] = [
  { answer: 'accepted', label: 'Accept' },
  { answer: 'tentative', label: 'Maybe' },
  { answer: 'declined', label: 'Decline' },
];

const DONE: Record<Answer, string> = {
  accepted: 'Accepted',
  tentative: 'Maybe',
  declined: 'Declined',
};

const SAID: Record<string, string> = {
  accepted: 'accepted',
  tentative: 'said maybe to',
  declined: 'declined',
  'needs-action': "hasn't answered",
};

function asEvent(view: InvitationView): CalendarEvent {
  return { ...view.event, id: '', calendarId: '', accountId: '', color: null };
}

/**
 * A meeting invitation in an email: what, when and where, and buttons to
 * answer it. Answering saves it in your calendar and (unless switched off)
 * emails the organiser. Cancellations and other people's answers show too.
 */
export function InvitationCard({ messageId, initial }: Props) {
  const api = useMailApi();
  const id = useId();
  const [view, setView] = useState(initial);
  const [notify, setNotify] = useState(true);
  const [busy, setBusy] = useState(false);
  // The answer being sent right now.
  const [sending, setSending] = useState<Answer | null>(null);
  // Who was told, after answering here.
  const [told, setTold] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  // What you chose here: a decline leaves nothing in the calendar to show it.
  const [chosen, setChosen] = useState<Answer | null>(null);
  const event = asEvent(view);
  const organizer = view.event.organizer;
  const answered = chosen ?? view.savedStatus ?? (view.myStatus !== 'needs-action' ? view.myStatus : null);

  const run = (call: () => Promise<InvitationView>, answer: Answer | null = null) => {
    setBusy(true);
    setError(null);
    call().then(
      (next) => {
        setBusy(false);
        setSending(null);
        setView(next);
        if (answer) setChosen(answer);
      },
      (e: unknown) => {
        setBusy(false);
        setSending(null);
        setError(toAppError(e));
      },
    );
  };

  const heading =
    view.method === 'CANCEL'
      ? 'Cancelled event'
      : view.method === 'REPLY'
        ? 'Answer to your invitation'
        : view.sequence > 0
          ? 'Updated invitation'
          : 'Invitation';

  return (
    <section
      className={`invite${view.method === 'CANCEL' ? ' invite--cancelled' : ''}`}
      aria-labelledby={`${id}-title`}
    >
      <p className="invite__kind">
        <Icon name="calendar" size={16} />
        {heading}
      </p>
      <h2 id={`${id}-title`} className="invite__title">
        {view.event.title || '(No title)'}
      </h2>
      <p className="invite__line">
        <Icon name="clock" size={16} />
        {whenText(event)}
      </p>
      {view.event.recurring && (
        <p className="invite__line">
          <Icon name="repeat" size={16} />
          {repeatText(view.event.repeat, new Date(view.event.start * 1000))}
        </p>
      )}
      {view.event.location && (
        <p className="invite__line">
          <Icon name="pin" size={16} />
          {view.event.location}
        </p>
      )}
      {organizer && (
        <p className="invite__line muted">
          <Icon name="people" size={16} />
          From {organizer.name ?? organizer.email}
          {view.event.attendees.length > 1 ? `, with ${String(view.event.attendees.length - 1)} others` : ''}
        </p>
      )}

      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}

      {view.method === 'REQUEST' && (
        <>
          {chosen ? (
            <p className="invite__done" role="status">
              <Icon name="check" size={16} />
              {DONE[chosen]}. {told ? `${told} has been told.` : 'Nobody was told.'}
              {chosen !== 'declined' && view.calendarEventId ? ' It is in your calendar.' : ''}
            </p>
          ) : (
            answered && (
              <p className="invite__status" role="status">
                You {SAID[answered] ?? answered} this
                {answered !== 'declined' && view.calendarEventId ? '; it is in your calendar.' : '.'}
              </p>
            )
          )}
          <div className="invite__actions" role="group" aria-label="Answer">
            {ANSWERS.map(({ answer, label }) => {
              const current = answered === answer;
              return (
                <button
                  key={answer}
                  type="button"
                  className={`btn ${current ? 'btn--primary' : 'btn--secondary'} btn--small invite__answer`}
                  aria-pressed={current}
                  // The same answer twice would tell the organiser twice.
                  disabled={busy || current}
                  onClick={() => {
                    const telling = notify && organizer !== null;
                    setSending(answer);
                    setTold(telling ? (organizer.name ?? organizer.email) : null);
                    run(() => api.answerInvitation(messageId, answer, telling), answer);
                  }}
                >
                  {current && <Icon name="check" size={14} />}
                  {sending === answer ? 'Sending…' : current ? DONE[answer] : label}
                </button>
              );
            })}
          </div>
          {organizer && (
            <label className="choice invite__notify">
              <input
                type="checkbox"
                checked={notify}
                onChange={(e) => {
                  setNotify(e.target.checked);
                }}
              />
              Send my answer to {organizer.name ?? organizer.email}
            </label>
          )}
          {!view.hasCalendar && (
            <p className="form-hint">
              This account has no calendar here, so the event won't be saved; only your answer is sent.
            </p>
          )}
        </>
      )}

      {view.method === 'CANCEL' &&
        (view.calendarEventId ? (
          <div className="invite__actions">
            <button
              type="button"
              className="btn btn--secondary btn--small"
              disabled={busy}
              onClick={() => {
                run(() => api.removeCancelledEvent(messageId));
              }}
            >
              Remove from my calendar
            </button>
          </div>
        ) : (
          <p className="invite__status" role="status">
            The organiser cancelled this. It isn't in your calendar.
          </p>
        ))}

      {view.method === 'REPLY' && view.replier && (
        <>
          <p className="invite__status" role="status">
            {view.replier.name ?? view.replier.email} {SAID[view.replier.status ?? ''] ?? 'answered'} your invitation.
          </p>
          {view.replyPending && (
            <div className="invite__actions">
              <button
                type="button"
                className="btn btn--secondary btn--small"
                disabled={busy}
                onClick={() => {
                  run(() => api.applyInvitationReply(messageId));
                }}
              >
                Update my calendar
              </button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
