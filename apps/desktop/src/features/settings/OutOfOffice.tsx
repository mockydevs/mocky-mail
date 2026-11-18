import { useEffect, useId, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, OutOfOffice as Status, Vacation } from '../../api/types';

interface Props {
  accounts: readonly Account[];
}

const day = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short' });

/** "until Mon 4 Jan" from `YYYY-MM-DD`. */
function dayText(key: string): string {
  const [y = 1970, m = 1, d = 1] = key.split('-').map(Number);
  return day.format(new Date(y, m - 1, d));
}

function summary(status: Status): string {
  const { start, end } = status.settings;
  if (!status.active) return 'Automatic replies are off.';
  if (start && end) return `Automatic replies are on from ${dayText(start)} to ${dayText(end)}.`;
  if (end) return `Automatic replies are on until ${dayText(end)}.`;
  if (start) return `Automatic replies are on from ${dayText(start)}.`;
  return 'Automatic replies are on.';
}

/**
 * Automatic replies (out of office) for one account at a time, saved on
 * the mail server so they go out while every device is off.
 */
export function OutOfOffice({ accounts }: Props) {
  const id = useId();
  const [accountId, setAccountId] = useState(accounts[0]?.id ?? '');
  return (
    <section className="settings__group" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="settings__legend">
        Automatic replies
      </h2>
      {accounts.length > 1 && (
        <label className="form-field">
          <span className="form-field__label">Account</span>
          <select
            className="field"
            value={accountId}
            onChange={(e) => {
              setAccountId(e.target.value);
            }}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.displayName ? `${a.displayName} (${a.email})` : a.email}
              </option>
            ))}
          </select>
        </label>
      )}
      {accountId && <AccountReplies key={accountId} accountId={accountId} />}
    </section>
  );
}

/** One account's automatic replies. */
function AccountReplies({ accountId }: { accountId: string }) {
  const api = useMailApi();
  const [status, setStatus] = useState<Status | null>(null);
  const [form, setForm] = useState<Vacation | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let live = true;
    api.getOutOfOffice(accountId).then(
      (next) => {
        if (!live) return;
        setStatus(next);
        setForm(next.settings);
      },
      (e: unknown) => {
        if (live) setError(toAppError(e));
      },
    );
    return () => {
      live = false;
    };
  }, [api, accountId]);

  const set = <K extends keyof Vacation>(key: K, value: Vacation[K]) => {
    setSaved(false);
    setForm((f) => (f ? { ...f, [key]: value } : f));
  };

  const save = (e: SyntheticEvent) => {
    e.preventDefault();
    if (!form) return;
    setBusy(true);
    setError(null);
    api.setOutOfOffice(accountId, form).then(
      (next) => {
        setBusy(false);
        setStatus(next);
        setForm(next.settings);
        setSaved(true);
      },
      (err: unknown) => {
        setBusy(false);
        setError(toAppError(err));
      },
    );
  };

  return (
    <>
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {!status && !error && (
        <p className="muted" role="status">
          Checking the mail server…
        </p>
      )}
      {status && !status.available && <p className="muted">{status.message}</p>}
      {status?.available && form && (
        <form className="ooo" aria-label="Automatic replies" onSubmit={save}>
          <p className={`ooo__status${status.active ? ' ooo__status--on' : ''}`} role="status">
            {saved ? 'Saved. ' : ''}
            {summary(status)}
          </p>
          <label className="choice">
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(e) => {
                set('enabled', e.target.checked);
              }}
            />
            Send automatic replies
          </label>
          <div className="ooo__dates">
            <label className="form-field">
              <span className="form-field__label">From (optional)</span>
              <input
                className="field"
                type="date"
                value={form.start ?? ''}
                onChange={(e) => {
                  set('start', e.target.value || null);
                }}
              />
            </label>
            <label className="form-field">
              <span className="form-field__label">Until (optional)</span>
              <input
                className="field"
                type="date"
                min={form.start ?? undefined}
                value={form.end ?? ''}
                onChange={(e) => {
                  set('end', e.target.value || null);
                }}
              />
            </label>
          </div>
          <label className="form-field">
            <span className="form-field__label">Subject</span>
            <input
              className="field"
              value={form.subject}
              maxLength={200}
              onChange={(e) => {
                set('subject', e.target.value);
              }}
            />
          </label>
          <label className="form-field">
            <span className="form-field__label">Message</span>
            <textarea
              className="field ooo__message"
              rows={5}
              value={form.message}
              placeholder="Thanks for your email. I'm away and will reply when I'm back."
              onChange={(e) => {
                set('message', e.target.value);
              }}
            />
          </label>
          <label className="form-field">
            <span className="form-field__label">Reply to the same person</span>
            <select
              className="field"
              value={form.days}
              onChange={(e) => {
                set('days', Number(e.target.value));
              }}
            >
              <option value={1}>At most once a day</option>
              <option value={3}>At most every 3 days</option>
              <option value={7}>At most once a week</option>
            </select>
          </label>
          <p className="form-hint">
            Your mail server sends these, even when this device is off. Mailing lists and automatic emails get no reply.
          </p>
          <div>
            <button type="submit" className="btn btn--primary" disabled={busy}>
              {busy ? 'Saving…' : 'Save automatic replies'}
            </button>
          </div>
        </form>
      )}
    </>
  );
}
