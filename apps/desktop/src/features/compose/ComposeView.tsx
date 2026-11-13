import { useId, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import type { Account } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { parseRecipients } from './recipients';
import type { ComposeSeed } from './seed';

interface Props {
  seed: ComposeSeed;
  accounts: readonly Account[];
  offlineNote: boolean;
  onClose: () => void;
  onQueued: () => void;
}

interface Errors {
  to?: string;
  cc?: string;
  bcc?: string;
}

function recipientError(input: string, required: boolean): string | undefined {
  const { valid, invalid } = parseRecipients(input);
  if (invalid.length > 0) return `Check ${invalid.join(', ')}. That does not look like an email address.`;
  if (required && valid.length === 0) return 'Add at least one recipient.';
  return undefined;
}

export function ComposeView({ seed, accounts, offlineNote, onClose, onQueued }: Props) {
  const api = useMailApi();
  const ids = useId();
  const [accountId, setAccountId] = useState(seed.accountId);
  const [to, setTo] = useState(seed.to);
  const [cc, setCc] = useState('');
  const [bcc, setBcc] = useState('');
  const [showCopies, setShowCopies] = useState(false);
  const [subject, setSubject] = useState(seed.subject);
  const [body, setBody] = useState(seed.body);
  const [errors, setErrors] = useState<Errors>({});
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const nextErrors: Errors = {};
    const toError = recipientError(to, true);
    const ccError = recipientError(cc, false);
    const bccError = recipientError(bcc, false);
    if (toError) nextErrors.to = toError;
    if (ccError) nextErrors.cc = ccError;
    if (bccError) nextErrors.bcc = bccError;
    setErrors(nextErrors);
    if (toError ?? ccError ?? bccError) return;

    setSending(true);
    setFailure(null);
    const replaced = seed.replacesOutboxId;
    void api
      .queueMessage({
        accountId,
        to: parseRecipients(to).valid,
        cc: parseRecipients(cc).valid,
        bcc: parseRecipients(bcc).valid,
        subject: subject.trim(),
        body,
      })
      .then(() => (replaced ? api.outbox.cancel(replaced) : undefined))
      .then(onQueued, (error: unknown) => {
        setFailure(error instanceof Error ? error.message : 'The message could not be queued.');
        setSending(false);
      });
  };

  const field = (name: keyof Errors, label: string, value: string, set: (v: string) => void, autoFocus = false) => {
    const id = `${ids}-${name}`;
    const errorId = `${id}-error`;
    const error = errors[name];
    return (
      <div className="compose__row">
        <label htmlFor={id} className="compose__label">
          {label}
        </label>
        <div className="compose__field">
          <input
            id={id}
            type="text"
            inputMode="email"
            autoComplete="email"
            className="compose__input"
            value={value}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
            // The first field of a dialog the user just opened.
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus={autoFocus}
            onChange={(e) => {
              set(e.target.value);
            }}
          />
          {error && (
            <p id={errorId} className="compose__error" role="alert">
              {error}
            </p>
          )}
        </div>
      </div>
    );
  };

  return (
    <form className="compose" aria-labelledby={`${ids}-title`} onSubmit={submit} noValidate>
      <header className="compose__head">
        <h1 id={`${ids}-title`} className="compose__title">
          {seed.replacesOutboxId ? 'Edit message' : 'New message'}
        </h1>
        <button type="button" className="icon-btn compose__close" aria-label="Close and discard" onClick={onClose}>
          <Icon name="close" />
        </button>
      </header>

      <div className="compose__fields">
        <div className="compose__row">
          <label htmlFor={`${ids}-from`} className="compose__label">
            From
          </label>
          <select
            id={`${ids}-from`}
            className="field field--select"
            value={accountId}
            onChange={(e) => {
              setAccountId(e.target.value);
            }}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label}, {a.address}
              </option>
            ))}
          </select>
        </div>
        {field('to', 'To', to, setTo, seed.to === '')}
        {!showCopies && (
          <div className="compose__row compose__row--copies">
            <button
              type="button"
              className="btn btn--link"
              onClick={() => {
                setShowCopies(true);
              }}
            >
              Add Cc or Bcc
            </button>
          </div>
        )}
        {showCopies && field('cc', 'Cc', cc, setCc)}
        {showCopies && field('bcc', 'Bcc', bcc, setBcc)}
        <div className="compose__row">
          <label htmlFor={`${ids}-subject`} className="compose__label">
            Subject
          </label>
          <input
            id={`${ids}-subject`}
            type="text"
            className="compose__input compose__input--subject"
            value={subject}
            onChange={(e) => {
              setSubject(e.target.value);
            }}
          />
        </div>
      </div>

      <label htmlFor={`${ids}-body`} className="visually-hidden">
        Message
      </label>
      <textarea
        id={`${ids}-body`}
        className="compose__body"
        value={body}
        // Replies start in the body, since To is already filled.
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={seed.to !== ''}
        onChange={(e) => {
          setBody(e.target.value);
        }}
      />

      <footer className="compose__foot">
        <button type="submit" className="btn btn--primary btn--large" disabled={sending}>
          <Icon name="sent" size={16} />
          {sending ? 'Sending' : 'Send'}
        </button>
        {offlineNote && (
          <span className="compose__note">
            <Icon name="clock" size={16} />
            If the connection drops, this waits in Outbox and sends by itself.
          </span>
        )}
        {failure && (
          <p className="compose__error" role="alert">
            {failure}
          </p>
        )}
        <span className="statusbar__spacer" />
        <button type="button" className="icon-btn" aria-label="Discard" onClick={onClose}>
          <Icon name="trash" />
        </button>
      </footer>
    </form>
  );
}
