import { useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Security, ServerConfig, ServerSuggestion } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { PasswordField } from '../../ui/PasswordField';
import { parseRecipients } from '../compose/recipients';
import { AdminConsentPanel } from './AdminConsent';
import { ExchangeForm } from './ExchangeForm';
import { ProviderLogo } from './ProviderLogo';
import { accountErrorCopy, appPasswordProvider, domainOf, guessServers, isValidServer, serverErrorCopy } from './providers';

interface Props {
  onAdded: (account: Account) => void;
  onCancel?: () => void;
}

type Busy = null | 'password' | 'google' | 'microsoft';

/** A problem with something typed, shown under the field it is about. */
interface FieldError {
  field: 'email' | 'password' | 'servers';
  message: string;
}

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
          <span className="form-field__label">Server name</span>
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
 *
 * Server settings are looked up once per domain and shown under the address,
 * with Edit to change them. Until the user edits them they always follow the
 * address typed, and Add account waits
 * for a lookup that is still under way instead of sending a guess.
 */
export function AddAccountForm({ onAdded, onCancel }: Props) {
  const api = useMailApi();
  const id = useId();
  const passwordRef = useRef<HTMLInputElement>(null);
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [imap, setImap] = useState<ServerConfig | null>(null);
  const [smtp, setSmtp] = useState<ServerConfig | null>(null);
  const [looking, setLooking] = useState(false);
  // The lookup for the domain typed, shared by blur, Advanced and submit.
  const lookup = useRef<{ domain: string; result: Promise<ServerSuggestion | null> } | null>(null);
  // Set once the user changes a server field: their settings then win.
  const serversEdited = useRef(false);
  const [suggestedOAuth, setSuggestedOAuth] = useState<'google' | 'microsoft' | null>(null);
  const [noSuggestion, setNoSuggestion] = useState(false);
  /** The lookup says this domain is on an Exchange Server. */
  const [suggestedExchange, setSuggestedExchange] = useState(false);
  /** The Exchange Server form is shown instead. */
  const [exchange, setExchange] = useState(false);
  /** The provider last tried, for Try again after an administrator approves the app. */
  const [lastOAuth, setLastOAuth] = useState<'google' | 'microsoft'>('microsoft');
  const [advanced, setAdvanced] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  // Which sign-in failed: the message goes next to that part of the form.
  const [errorFrom, setErrorFrom] = useState<'password' | 'oauth'>('password');
  const [fieldError, setFieldError] = useState<FieldError | null>(null);
  const [busy, setBusy] = useState<Busy>(null);

  const emailValid = parseRecipients(email).valid.length === 1 && parseRecipients(email).invalid.length === 0;
  const provider = appPasswordProvider(email);
  const domain = domainOf(email);

  /** Settings for the address typed: the lookup under way, or a new one. */
  const findServers = (): Promise<ServerSuggestion | null> => {
    const current = lookup.current;
    if (current?.domain === domain) return current.result;
    const address = email.trim();
    let failed = false;
    const result = api.suggestServers(address).catch(() => {
      failed = true;
      return null;
    });
    lookup.current = { domain, result };
    setLooking(true);
    void result.then((suggestion) => {
      // The address moved to another domain meanwhile: this answer is stale.
      if (lookup.current?.result !== result) return;
      // A lookup that broke (not one that found nothing) may be tried again.
      if (failed) lookup.current = null;
      setLooking(false);
      const chosen = suggestion ?? guessServers(address);
      if (!serversEdited.current) {
        setImap(chosen.imap);
        setSmtp(chosen.smtp);
      }
      setSuggestedOAuth(suggestion && (suggestion.auth === 'google' || suggestion.auth === 'microsoft') ? suggestion.auth : null);
      setSuggestedExchange(suggestion?.auth === 'exchange');
      setNoSuggestion(suggestion === null);
      if (suggestion === null) setAdvanced(true);
    });
    return result;
  };

  const suggest = () => {
    if (emailValid) void findServers();
  };

  const changeEmail = (value: string) => {
    setEmail(value);
    // Same domain: the settings and sign-in suggestion found still apply.
    if (domainOf(value) === lookup.current?.domain) return;
    lookup.current = null;
    setLooking(false);
    setSuggestedOAuth(null);
    setSuggestedExchange(false);
    setNoSuggestion(false);
    if (!serversEdited.current) {
      setImap(null);
      setSmtp(null);
    }
  };

  const editServer = (set: (next: ServerConfig) => void) => (next: ServerConfig) => {
    serversEdited.current = true;
    set(next);
  };

  const fail = (from: 'password' | 'oauth') => (e: unknown) => {
    const failure = toAppError(e);
    setError(failure);
    setErrorFrom(from);
    setBusy(null);
    // Both of these usually mean the server name or port is wrong: show them.
    if (failure.kind === 'tls' || failure.kind === 'connect') setAdvanced(true);
  };

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const field = passwordRef.current;
    const password = field?.value ?? '';
    if (field) field.value = '';
    setError(null);
    if (!emailValid) {
      setFieldError({ field: 'email', message: 'Enter a valid email address.' });
      return;
    }
    if (!password) {
      setFieldError({ field: 'password', message: 'Enter your password or app password.' });
      return;
    }
    setFieldError(null);
    setBusy('password');
    const address = email.trim();
    // Settings the user typed are used as they are. Otherwise wait for the
    // lookup, so a quick Add account never goes out with a guessed server.
    const ready =
      serversEdited.current && imap && smtp
        ? Promise.resolve({ imap, smtp })
        : findServers().then((suggestion) => {
            const chosen = suggestion ?? guessServers(address);
            return { imap: chosen.imap, smtp: chosen.smtp };
          });
    void ready.then((servers) => {
      if (!isValidServer(servers.imap) || !isValidServer(servers.smtp)) {
        setImap(servers.imap);
        setSmtp(servers.smtp);
        setBusy(null);
        setAdvanced(true);
        setFieldError({ field: 'servers', message: 'Check the server names and ports.' });
        return;
      }
      api
        .addAccount({ email: address, displayName: displayName.trim(), password, ...servers })
        .then(onAdded, fail('password'));
    });
  };

  const oauth = (provider: 'google' | 'microsoft') => {
    setError(null);
    // No address needed: the provider shows its own account chooser. One
    // that was typed is passed on as a hint, so it must be valid.
    if (email.trim() !== '' && !emailValid) {
      setFieldError({ field: 'email', message: 'Check the email address, or clear it to choose an account.' });
      return;
    }
    setFieldError(null);
    setLastOAuth(provider);
    setBusy(provider);
    api.addOAuthAccount({ email: email.trim(), displayName: displayName.trim(), provider }).then(onAdded, fail('oauth'));
  };

  const toggleAdvanced = () => {
    if (!imap) setImap(guessServers(email).imap);
    if (!smtp) setSmtp(guessServers(email).smtp);
    // Opened before the address was looked up: fill in the real settings.
    if (emailValid) void findServers();
    setAdvanced((open) => !open);
  };

  // Each failure is shown next to what it is about: the password, the server
  // settings, or the sign-in button that was pressed.
  const passwordFailure = error && errorFrom === 'password' ? error : null;
  const authError = passwordFailure?.kind === 'auth' ? passwordFailure : null;
  const serverError =
    passwordFailure && (passwordFailure.kind === 'connect' || passwordFailure.kind === 'tls') ? passwordFailure : null;
  const otherError = error && !authError && !serverError ? error : null;
  const serverCopy = serverError ? serverErrorCopy(serverError) : null;

  const errorFor = (which: FieldError['field']) => (fieldError?.field === which ? fieldError.message : null);
  const emailError = errorFor('email');
  const passwordError = errorFor('password');
  const serversError = errorFor('servers');

  const notice = (from: 'password' | 'oauth') => {
    if (!otherError || errorFrom !== from) return null;
    if (from === 'oauth' && otherError.adminConsentUrl) {
      return (
        <AdminConsentPanel
          url={otherError.adminConsentUrl}
          onRetry={() => {
            oauth(lastOAuth);
          }}
          busy={busy === lastOAuth}
        />
      );
    }
    const copy = accountErrorCopy(otherError, email, from);
    return (
      <div className="notice notice--error" role="alert">
        <p className="notice__title">{copy.title}</p>
        <p>{copy.message}</p>
        {copy.hint && <p className="notice__hint">{copy.hint}</p>}
      </div>
    );
  };

  const oauthButton = (which: 'google' | 'microsoft', variant = 'btn--secondary') => (
    <button
      type="button"
      className={`btn ${variant} btn--large`}
      disabled={busy !== null}
      aria-busy={busy === which ? true : undefined}
      onClick={() => {
        oauth(which);
      }}
    >
      {busy === which ? <span className="spinner" aria-hidden="true" /> : <ProviderLogo provider={which} />}
      {busy === which ? `Waiting for ${providerName(which)}` : `Continue with ${providerName(which)}`}
    </button>
  );

  const signinProgress = (busy === 'google' || busy === 'microsoft') && (
    <p className="signin-progress" role="status">
      Signing in with {providerName(busy)}. Finish on the {providerName(busy)} screen; this page carries on by itself,
      then checks your mailbox.
    </p>
  );

  const editButton = (label: string, variant = 'btn--quiet') => (
    <button
      type="button"
      className={`btn btn--small ${variant}`}
      aria-expanded={advanced}
      aria-controls={`${id}-advanced`}
      disabled={busy !== null}
      onClick={toggleAdvanced}
    >
      {advanced ? 'Done' : label}
    </button>
  );

  const serverFields = advanced && imap && smtp && (
    <div id={`${id}-advanced`} className="advanced">
      <ServerFields legend="Incoming mail (IMAP)" value={imap} onChange={editServer(setImap)} />
      <ServerFields legend="Outgoing mail (SMTP)" value={smtp} onChange={editServer(setSmtp)} />
    </div>
  );

  const serverLine = imap && smtp && (
    <p className="server-line">
      <span>
        Incoming {imap.host}:{imap.port}
      </span>
      <span>
        Outgoing {smtp.host}:{smtp.port}
      </span>
    </p>
  );

  // The mail settings for the address: what is being looked up, what was
  // found, or what went wrong with it, always in the same place.
  const mailSettings = () => {
    const serversProblem = serversError && (
      <p className="form-error" role="alert">
        <Icon name="alert" />
        <span>{serversError}</span>
      </p>
    );
    if (serverError && serverCopy) {
      return (
        <div className="slot slot--error" role="alert">
          <div className="slot__row">
            <Icon name="alert" />
            <div className="slot__body">
              <p className="slot__title">{serverCopy.title}</p>
              <p className="slot__text">{serverCopy.message}</p>
              {!advanced && serverLine}
            </div>
          </div>
          <div className="slot__actions">{editButton('Edit server settings', 'btn--secondary')}</div>
          {serverFields}
          <details className="slot__details">
            <summary>Technical details</summary>
            <code>{serverError.message}</code>
          </details>
        </div>
      );
    }
    if (looking) {
      return (
        <div className="slot" role="status">
          <div className="slot__row">
            <span className="spinner" aria-hidden="true" />
            <div className="slot__body">
              <p className="slot__title">Finding your mail settings</p>
              <p className="slot__text">Looking up {domain}. This takes a moment.</p>
            </div>
          </div>
          {serverFields}
        </div>
      );
    }
    if (noSuggestion) {
      return (
        <div className="slot slot--warn" role="status">
          <div className="slot__row">
            <Icon name="alert" />
            <div className="slot__body">
              <p className="slot__title">We couldn’t find settings for {domain}</p>
              <p className="slot__text">
                We filled in the usual names below. Check them against your email provider’s help pages.
              </p>
            </div>
            {!advanced && editButton('Edit')}
          </div>
          {serverFields}
          {serversProblem}
        </div>
      );
    }
    const found = emailValid && imap !== null && smtp !== null;
    if (!found && !advanced) {
      return (
        <p className="slot-idle form-hint">
          Mail server settings are found from your address.{' '}
          <button
            type="button"
            className="btn--text"
            aria-expanded={false}
            aria-controls={`${id}-advanced`}
            disabled={busy !== null}
            onClick={toggleAdvanced}
          >
            Enter them myself
          </button>
        </p>
      );
    }
    return (
      <div className="slot" role={found ? 'status' : undefined}>
        <div className="slot__row">
          {found ? (
            <span className="slot__ok">
              <Icon name="check" />
            </span>
          ) : (
            <Icon name="mail" />
          )}
          <div className="slot__body">
            <p className="slot__title">{found ? 'Mail settings found' : 'Mail server settings'}</p>
            {found ? serverLine : <p className="slot__text">Your email provider lists these in its help pages.</p>}
          </div>
          {editButton('Edit')}
        </div>
        {serverFields}
        {serversProblem}
      </div>
    );
  };

  const passwordDescribedBy =
    [authError || passwordError ? `${id}-password-error` : null, provider ? `${id}-password-hint` : null]
      .filter(Boolean)
      .join(' ') || undefined;

  if (exchange) {
    return (
      <ExchangeForm
        onAdded={onAdded}
        onCancel={onCancel}
        initialEmail={email}
        onBack={(typed) => {
          if (typed !== email) changeEmail(typed);
          setExchange(false);
        }}
      />
    );
  }

  const exchangeButton = (
    <button
      type="button"
      className="btn btn--secondary btn--large"
      disabled={busy !== null}
      onClick={() => {
        setError(null);
        setExchange(true);
      }}
    >
      <Icon name="mail" />
      Exchange Server
    </button>
  );

  return (
    <form className="account-form" aria-labelledby={`${id}-title`} onSubmit={submit} noValidate>
      <header className="account-form__header">
        <h2 id={`${id}-title`} className="account-form__title">
          Add an email account
        </h2>
        <p className="form-hint">Works with Mocky Mail, Gmail, Outlook, Exchange Server and any other IMAP mailbox.</p>
      </header>

      {!suggestedOAuth && (
        <>
          <div className="oauth">
            {oauthButton('google')}
            {oauthButton('microsoft')}
            {exchangeButton}
          </div>
          {signinProgress}
          {notice('oauth')}
          <p className="divider">or use your email address</p>
        </>
      )}

      <div className="form-field">
        <label className="form-field__label" htmlFor={`${id}-email`}>
          Email address
        </label>
        <input
          id={`${id}-email`}
          className="field"
          type="email"
          autoComplete="email"
          placeholder="name@example.com"
          value={email}
          disabled={busy !== null}
          aria-invalid={emailError !== null ? true : undefined}
          aria-describedby={emailError !== null ? `${id}-email-error` : undefined}
          onChange={(e) => {
            changeEmail(e.target.value);
          }}
          onBlur={suggest}
        />
        {emailError !== null && (
          <p id={`${id}-email-error`} className="form-error" role="alert">
            <Icon name="alert" />
            <span>{emailError}</span>
          </p>
        )}
      </div>

      {suggestedExchange && !suggestedOAuth && (
        <div className="oauth-suggest" role="status">
          <p>This address is on an Exchange Server. Adding it as an Exchange Server account brings your mail, calendar and contacts.</p>
          <button
            type="button"
            className="btn btn--primary btn--block"
            disabled={busy !== null}
            onClick={() => {
              setExchange(true);
            }}
          >
            Add as Exchange Server
          </button>
        </div>
      )}

      {!suggestedOAuth && mailSettings()}

      {suggestedOAuth && (
        <>
          <div className="oauth-suggest" role="status">
            <p>
              {busy === suggestedOAuth
                ? `Finish signing in on the ${providerName(suggestedOAuth)} screen that just opened. This page carries on by itself, then checks your mailbox.`
                : `This is a ${providerName(suggestedOAuth)} account. Signing in with ${providerName(suggestedOAuth)} is quickest, with no password to type.`}
            </p>
            {oauthButton(suggestedOAuth, 'btn--primary btn--block')}
          </div>
          {notice('oauth')}
          <p className="divider">or use an app password</p>
        </>
      )}

      <div className="form-field">
        <label className="form-field__label" htmlFor={`${id}-password`}>
          {suggestedOAuth ? 'App password' : 'Password'}
        </label>
        <PasswordField
          id={`${id}-password`}
          inputRef={passwordRef}
          autoComplete="current-password"
          {...(passwordDescribedBy ? { describedBy: passwordDescribedBy } : {})}
          invalid={authError !== null || passwordError !== null}
        />
        {(authError ?? passwordError) && (
          <p id={`${id}-password-error`} className="form-error" role="alert">
            <Icon name="alert" />
            <span>
              {authError
                ? `That password wasn’t accepted. ${accountErrorCopy(authError, email).hint ?? 'Check it and try again.'}`
                : passwordError}
            </span>
          </p>
        )}
        {provider && (
          <p id={`${id}-password-hint`} className="form-hint">
            {provider} needs an app password here, not your normal password. Create one in your {provider} account
            security settings.
          </p>
        )}
      </div>

      {suggestedOAuth && mailSettings()}

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

      {notice('password')}

      <div className="account-form__actions">
        {onCancel && (
          <button type="button" className="btn btn--secondary btn--large" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button
          type="submit"
          className={`btn ${suggestedOAuth ? 'btn--secondary' : 'btn--primary'} btn--large account-form__submit`}
          disabled={busy !== null}
          aria-busy={busy === 'password' ? true : undefined}
        >
          {busy === 'password' && <span className="spinner" aria-hidden="true" />}
          {busy === 'password' ? 'Checking sign-in…' : suggestedOAuth ? 'Add with app password' : 'Add account'}
        </button>
      </div>
      {busy === 'password' && (
        <p className="signin-progress" role="status">
          {looking || !imap ? 'Finding your mail settings.' : `Connecting to ${imap.host}.`} This usually takes a few
          seconds.
        </p>
      )}

      <p className="account-form__foot">
        <Icon name="lock" size={15} />
        <span>Your password is kept in this device’s password store, never in plain text.</span>
      </p>
    </form>
  );
}
