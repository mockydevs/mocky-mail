import { useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type { Account, AppError } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { PasswordField } from '../../ui/PasswordField';
import { parseRecipients } from '../compose/recipients';

interface Props {
  onAdded: (account: Account) => void;
  /** Back to the other kinds of account, with the address typed here. */
  onBack: (email: string) => void;
  onCancel?: (() => void) | undefined;
  /** The address typed before choosing Exchange Server. */
  initialEmail?: string;
}

interface FieldError {
  field: 'email' | 'password' | 'server';
  message: string;
}

/**
 * What the server address field holds, ready to send: null for "find it
 * myself" (Autodiscover), the address, or why it can't be used. Only https
 * is accepted, so the password never travels unencrypted; a bare server name
 * is enough (the backend adds the rest).
 */
export function checkEwsUrl(value: string): { url: string | null } | { error: string } {
  const trimmed = value.trim();
  if (trimmed === '') return { url: null };
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(trimmed)?.[1]?.toLowerCase();
  if (scheme !== undefined && scheme !== 'https') {
    return {
      error: 'Use an address that starts with https://. Mocky Mail never sends your password over an unencrypted connection.',
    };
  }
  let host = '';
  try {
    host = new URL(scheme ? trimmed : `https://${trimmed}`).hostname;
  } catch {
    host = '';
  }
  if (!/^[a-z0-9.-]+$/i.test(host) || !host.includes('.') || /\s/.test(trimmed)) {
    return { error: 'Check the server address, for example https://mail.example.com/EWS/Exchange.asmx.' };
  }
  return { url: trimmed };
}

function hostOf(url: string): string {
  try {
    return new URL(/^https:/i.test(url) ? url : `https://${url}`).hostname;
  } catch {
    return url;
  }
}

/**
 * An account on an organisation's own Exchange Server: address, password and,
 * when the organisation uses one, a sign-in name such as DOMAIN\name. The
 * server is found from the address (Autodiscover) unless entered by hand.
 * As in the main form, the password is read from its field on submit and the
 * field is cleared at once.
 */
export function ExchangeForm({ onAdded, onBack, onCancel, initialEmail = '' }: Props) {
  const api = useMailApi();
  const id = useId();
  const passwordRef = useRef<HTMLInputElement>(null);
  const [email, setEmail] = useState(initialEmail);
  const [displayName, setDisplayName] = useState('');
  const [username, setUsername] = useState('');
  const [manual, setManual] = useState(false);
  const [server, setServer] = useState('');
  const [busy, setBusy] = useState<null | { url: string | null }>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [fieldError, setFieldError] = useState<FieldError | null>(null);

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const field = passwordRef.current;
    const password = field?.value ?? '';
    if (field) field.value = '';
    setError(null);
    const parsed = parseRecipients(email);
    if (parsed.valid.length !== 1 || parsed.invalid.length > 0) {
      setFieldError({ field: 'email', message: 'Enter a valid email address.' });
      return;
    }
    if (!password) {
      setFieldError({ field: 'password', message: 'Enter your password.' });
      return;
    }
    const checked = manual ? checkEwsUrl(server) : { url: null };
    if ('error' in checked) {
      setFieldError({ field: 'server', message: checked.error });
      return;
    }
    setFieldError(null);
    setBusy({ url: checked.url });
    api
      .addExchangeAccount({
        email: email.trim(),
        displayName: displayName.trim(),
        username: username.trim(),
        password,
        ewsUrl: checked.url,
      })
      .then(onAdded, (e: unknown) => {
        const failure = toAppError(e);
        setBusy(null);
        setError(failure);
        // No server found, or none reachable: the address is what to check.
        if (failure.kind === 'config' || failure.kind === 'connect' || failure.kind === 'tls') setManual(true);
      });
  };

  const errorFor = (which: FieldError['field']) => (fieldError?.field === which ? fieldError.message : null);
  const emailError = errorFor('email');
  const passwordError =
    errorFor('password') ??
    (error?.kind === 'auth'
      ? 'That sign-in wasn’t accepted. Check the password, and the sign-in name if your organisation uses one like DOMAIN\\name.'
      : null);
  const serverError = errorFor('server');
  const serverProblem =
    error && (error.kind === 'config' || error.kind === 'connect' || error.kind === 'tls')
      ? {
          title: error.kind === 'config' ? 'We couldn’t find your Exchange server' : 'We couldn’t reach the Exchange server',
          text:
            error.kind === 'config'
              ? 'Enter the server address your IT department gives you below. It often ends in /EWS/Exchange.asmx.'
              : 'Check your internet connection and the server address below. Some Exchange servers can only be reached from the office network or a VPN.',
        }
      : null;
  const otherError = error && error.kind !== 'auth' && !serverProblem ? error : null;
  const progress = busy
    ? busy.url === null
      ? 'Finding your Exchange server…'
      : `Connecting to ${hostOf(busy.url)}…`
    : null;

  const fieldErrorText = (fid: string, text: string | null) =>
    text !== null && (
      <p id={fid} className="form-error" role="alert">
        <Icon name="alert" />
        <span>{text}</span>
      </p>
    );

  return (
    <form className="account-form" aria-labelledby={`${id}-title`} onSubmit={submit} noValidate>
      <header className="account-form__header">
        <h2 id={`${id}-title`} className="account-form__title">
          Add an Exchange Server account
        </h2>
        <p className="form-hint">
          For mailboxes on your organisation’s own Exchange Server. For Microsoft 365 or Outlook.com, go back and use
          Continue with Microsoft.
        </p>
      </header>

      <div className="form-field">
        <label className="form-field__label" htmlFor={`${id}-email`}>
          Email address
        </label>
        <input
          id={`${id}-email`}
          className="field"
          type="email"
          autoComplete="email"
          placeholder="name@company.com"
          value={email}
          disabled={busy !== null}
          aria-invalid={emailError !== null || undefined}
          aria-describedby={emailError !== null ? `${id}-email-error` : undefined}
          onChange={(e) => {
            setEmail(e.target.value);
          }}
        />
        {fieldErrorText(`${id}-email-error`, emailError)}
      </div>

      <label className="form-field" htmlFor={`${id}-name`}>
        <span className="form-field__label">Your name (optional)</span>
        <input
          id={`${id}-name`}
          className="field"
          autoComplete="name"
          placeholder="Shown to people you email"
          value={displayName}
          disabled={busy !== null}
          onChange={(e) => {
            setDisplayName(e.target.value);
          }}
        />
      </label>

      <div className="form-field">
        <label className="form-field__label" htmlFor={`${id}-password`}>
          Password
        </label>
        <PasswordField
          id={`${id}-password`}
          inputRef={passwordRef}
          autoComplete="current-password"
          {...(passwordError !== null ? { describedBy: `${id}-password-error` } : {})}
          invalid={passwordError !== null}
        />
        {fieldErrorText(`${id}-password-error`, passwordError)}
      </div>

      <label className="form-field" htmlFor={`${id}-user`}>
        <span className="form-field__label">Sign-in name (if not your email address, e.g. DOMAIN\name)</span>
        <input
          id={`${id}-user`}
          className="field"
          autoComplete="username"
          spellCheck={false}
          value={username}
          disabled={busy !== null}
          onChange={(e) => {
            setUsername(e.target.value);
          }}
        />
      </label>

      {serverProblem && (
        <div className="slot slot--error" role="alert">
          <div className="slot__row">
            <Icon name="alert" />
            <div className="slot__body">
              <p className="slot__title">{serverProblem.title}</p>
              <p className="slot__text">{serverProblem.text}</p>
            </div>
          </div>
          {error && (
            <details className="slot__details">
              <summary>Technical details</summary>
              <code>{error.message}</code>
            </details>
          )}
        </div>
      )}

      <p className="slot-idle form-hint">
        Your Exchange server is found from your address.{' '}
        <button
          type="button"
          className="btn--text"
          aria-expanded={manual}
          aria-controls={`${id}-server-box`}
          disabled={busy !== null}
          onClick={() => {
            setManual((open) => !open);
          }}
        >
          Enter the server address myself
        </button>
      </p>
      {manual && (
        <div id={`${id}-server-box`} className="form-field">
          <label className="form-field__label" htmlFor={`${id}-server`}>
            Server address
          </label>
          <input
            id={`${id}-server`}
            className="field"
            inputMode="url"
            spellCheck={false}
            autoComplete="off"
            placeholder="https://mail.example.com/EWS/Exchange.asmx"
            value={server}
            disabled={busy !== null}
            aria-invalid={serverError !== null || undefined}
            aria-describedby={serverError !== null ? `${id}-server-error` : `${id}-server-hint`}
            onChange={(e) => {
              setServer(e.target.value);
            }}
          />
          {fieldErrorText(`${id}-server-error`, serverError)}
          {serverError === null && (
            <p id={`${id}-server-hint`} className="form-hint">
              Your IT department can give you this. The server name alone is often enough. Leave it empty to find it
              from your address.
            </p>
          )}
        </div>
      )}

      {otherError && (
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{errorTitle(otherError.kind)}</p>
          <p>{otherError.message}</p>
        </div>
      )}

      <div className="account-form__actions">
        <button type="button" className="btn btn--secondary btn--large" disabled={busy !== null}
          onClick={() => {
            onBack(email);
          }}
        >
          Back
        </button>
        {onCancel && (
          <button type="button" className="btn btn--secondary btn--large" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button
          type="submit"
          className="btn btn--primary btn--large account-form__submit"
          disabled={busy !== null}
          aria-busy={busy !== null || undefined}
        >
          {busy && <span className="spinner" aria-hidden="true" />}
          {busy ? 'Checking sign-in…' : 'Add Exchange account'}
        </button>
      </div>
      {progress && (
        <p className="signin-progress" role="status">
          {progress} This can take up to a minute.
        </p>
      )}

      <p className="account-form__foot">
        <Icon name="lock" size={15} />
        <span>Your password is kept in this device’s password store, never in plain text. It is only sent over https.</span>
      </p>
    </form>
  );
}
