import { useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Security, ServerConfig } from '../../api/types';
import { PasswordField } from '../../ui/PasswordField';
import { parseRecipients } from '../compose/recipients';
import { accountErrorCopy, appPasswordProvider, domainOf, guessServers, isValidServer } from './providers';

interface Props {
  onAdded: (account: Account) => void;
  onCancel?: () => void;
}

type Busy = null | 'password' | 'google' | 'microsoft';

const providerName = (which: 'google' | 'microsoft') => (which === 'google' ? 'Google' : 'Microsoft');

function ServerFields({
  legend,
  value,
  onChange,
}: {
  legend: string;
  value: ServerConfig;
  onChange: (next: ServerConfig) => void;
}) {
  const id = useId();
  return (
    <fieldset className="server">
      <legend className="server__legend">{legend}</legend>
      <div className="server__grid">
        <label className="form-field" htmlFor={`${id}-host`}>
          <span className="form-field__label">Server</span>
          <input
            id={`${id}-host`}
            className="field"
            value={value.host}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => {
              onChange({ ...value, host: e.target.value.trim() });
            }}
          />
        </label>
        <label className="form-field" htmlFor={`${id}-port`}>
          <span className="form-field__label">Port</span>
          <input
            id={`${id}-port`}
            className="field"
            inputMode="numeric"
            value={String(value.port)}
            onChange={(e) => {
              const port = Number(e.target.value.replace(/\D/g, ''));
              onChange({ ...value, port });
            }}
          />
        </label>
        <label className="form-field" htmlFor={`${id}-security`}>
          <span className="form-field__label">Security</span>
          <select
            id={`${id}-security`}
            className="field field--select"
            value={value.security}
            onChange={(e) => {
              onChange({ ...value, security: e.target.value as Security });
            }}
          >
            <option value="tls">TLS</option>
            <option value="starttls">STARTTLS</option>
          </select>
        </label>
      </div>
    </fieldset>
  );
}

/**
 * Google and Microsoft sign-in come first: one tap opens the provider's
 * account chooser, with nothing to type. Below, any other mailbox: email,
 * password, name, then Add account. Typing a Gmail or Outlook address moves
 * that provider's sign-in under the email instead. The password is read from
 * the field on submit and the field is cleared at once; it is never kept in
 * component state.
 */
export function AddAccountForm({ onAdded, onCancel }: Props) {
  const api = useMailApi();
  const id = useId();
  const passwordRef = useRef<HTMLInputElement>(null);
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [imap, setImap] = useState<ServerConfig | null>(null);
  const [smtp, setSmtp] = useState<ServerConfig | null>(null);
  const [suggestedFor, setSuggestedFor] = useState<string | null>(null);
  const [suggestedOAuth, setSuggestedOAuth] = useState<'google' | 'microsoft' | null>(null);
  const [noSuggestion, setNoSuggestion] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Busy>(null);

  const emailValid = parseRecipients(email).valid.length === 1 && parseRecipients(email).invalid.length === 0;
  const provider = appPasswordProvider(email);
  const domain = domainOf(email);

  const suggest = () => {
    if (!emailValid || suggestedFor === domain) return;
    setSuggestedFor(domain);
    void api.suggestServers(email.trim()).then(
      (suggestion) => {
        const chosen = suggestion ?? guessServers(email);
        setImap((current) => current ?? chosen.imap);
        setSmtp((current) => current ?? chosen.smtp);
        setSuggestedOAuth(suggestion && suggestion.auth !== 'password' ? suggestion.auth : null);
        setNoSuggestion(suggestion === null);
        if (suggestion === null) setAdvanced(true);
      },
      () => {
        const guess = guessServers(email);
        setImap((current) => current ?? guess.imap);
        setSmtp((current) => current ?? guess.smtp);
        setNoSuggestion(true);
        setAdvanced(true);
      },
    );
  };

  const fail = (e: unknown) => {
    setError(toAppError(e));
    setBusy(null);
  };

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const field = passwordRef.current;
    const password = field?.value ?? '';
    if (field) field.value = '';
    setError(null);
    if (!emailValid) {
      setFieldError('Enter a valid email address.');
      return;
    }
    if (!password) {
      setFieldError('Enter your password or app password.');
      return;
    }
    const servers = { imap: imap ?? guessServers(email).imap, smtp: smtp ?? guessServers(email).smtp };
    if (!isValidServer(servers.imap) || !isValidServer(servers.smtp)) {
      setAdvanced(true);
      setFieldError('Check the server names and ports under Advanced.');
      return;
    }
    setFieldError(null);
    setBusy('password');
    api
      .addAccount({ email: email.trim(), displayName: displayName.trim(), password, ...servers })
      .then(onAdded, fail);
  };

  const oauth = (provider: 'google' | 'microsoft') => {
    setError(null);
    // No address needed: the provider shows its own account chooser. One
    // that was typed is passed on as a hint, so it must be valid.
    if (email.trim() !== '' && !emailValid) {
      setFieldError('Check the email address, or clear it to choose an account.');
      return;
    }
    setFieldError(null);
    setBusy(provider);
    api.addOAuthAccount({ email: email.trim(), displayName: displayName.trim(), provider }).then(onAdded, fail);
  };

  const copy = error ? accountErrorCopy(error, email) : null;

  const oauthButton = (which: 'google' | 'microsoft', variant = 'btn--secondary') => (
    <button
      type="button"
      className={`btn ${variant} btn--large`}
      disabled={busy !== null}
      onClick={() => {
        oauth(which);
      }}
    >
      {busy === which ? `Waiting for ${providerName(which)}` : `Continue with ${providerName(which)}`}
    </button>
  );

  return (
    <form className="account-form" aria-labelledby={`${id}-title`} onSubmit={submit} noValidate>
      <header className="account-form__header">
        <h2 id={`${id}-title`} className="account-form__title">
          Add an email account
        </h2>
        <p className="form-hint">Works with Mocky Mail, Gmail, Outlook and any other IMAP mailbox.</p>
      </header>

      {copy && (
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{copy.title}</p>
          <p>{copy.message}</p>
          {copy.hint && <p className="notice__hint">{copy.hint}</p>}
        </div>
      )}

      {!suggestedOAuth && (
        <>
          <div className="oauth">
            {oauthButton('google')}
            {oauthButton('microsoft')}
          </div>
          <p className="divider">or add any email account</p>
        </>
      )}

      <label className="form-field" htmlFor={`${id}-email`}>
        <span className="form-field__label">Email address</span>
        <input
          id={`${id}-email`}
          className="field"
          type="email"
          autoComplete="email"
          value={email}
          aria-invalid={fieldError !== null && !emailValid ? true : undefined}
          onChange={(e) => {
            setEmail(e.target.value);
            setSuggestedOAuth(null);
            setNoSuggestion(false);
          }}
          onBlur={suggest}
        />
      </label>

      {suggestedOAuth && (
        <div className="oauth-suggest" role="status">
          <p className="form-hint">
            This is a {providerName(suggestedOAuth)} account. Signing in with {providerName(suggestedOAuth)} is
            quickest, with no password to type.
          </p>
          {oauthButton(suggestedOAuth, 'btn--primary btn--block')}
          <p className="divider">or use an app password</p>
        </div>
      )}

      <div className="form-field">
        <label className="form-field__label" htmlFor={`${id}-password`}>
          Password or app password
        </label>
        <PasswordField
          id={`${id}-password`}
          inputRef={passwordRef}
          autoComplete="current-password"
          describedBy={`${id}-password-hint`}
        />
      </div>
      <p id={`${id}-password-hint`} className="form-hint">
        {provider
          ? `${provider} needs an app password here, not your normal password. Create one in your ${provider} account security settings.`
          : 'Gmail, Yahoo and iCloud need an app password for mail apps. Your password is kept in the system password store, never in plain text.'}
      </p>

      <label className="form-field" htmlFor={`${id}-name`}>
        <span className="form-field__label">Your name (optional)</span>
        <input
          id={`${id}-name`}
          className="field"
          autoComplete="name"
          placeholder="Shown to people you email"
          value={displayName}
          onChange={(e) => {
            setDisplayName(e.target.value);
          }}
        />
      </label>

      {noSuggestion && (
        <p className="form-hint" role="status">
          We could not find settings for {domain}. Check the servers below; your email provider lists them in their help pages.
        </p>
      )}

      <button
        type="button"
        className="btn btn--link advanced-toggle"
        aria-expanded={advanced}
        aria-controls={`${id}-advanced`}
        onClick={() => {
          if (!imap) setImap(guessServers(email).imap);
          if (!smtp) setSmtp(guessServers(email).smtp);
          setAdvanced((open) => !open);
        }}
      >
        Advanced: server settings
      </button>
      {advanced && imap && smtp && (
        <div id={`${id}-advanced`} className="advanced">
          <ServerFields legend="Incoming mail (IMAP)" value={imap} onChange={setImap} />
          <ServerFields legend="Outgoing mail (SMTP)" value={smtp} onChange={setSmtp} />
        </div>
      )}

      {fieldError && (
        <p className="form-error" role="alert">
          {fieldError}
        </p>
      )}

      <div className="account-form__actions">
        {onCancel && (
          <button type="button" className="btn btn--secondary" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button type="submit" className={`btn ${suggestedOAuth ? 'btn--secondary' : 'btn--primary'} btn--large account-form__submit`} disabled={busy !== null}>
          {busy === 'password' ? 'Checking sign-in' : 'Add account'}
        </button>
      </div>

    </form>
  );
}
