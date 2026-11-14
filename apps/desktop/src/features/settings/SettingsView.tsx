import { useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Settings, ThemeSetting } from '../../api/types';
import { ConfirmDialog } from '../../ui/Modal';
import { Icon } from '../../ui/Icon';
import { PasswordField } from '../../ui/PasswordField';
import { playChime } from '../notifications/chime';
import { accountErrorCopy } from '../accounts/providers';

interface Props {
  settings: Settings;
  accounts: readonly Account[];
  onChange: (next: Settings) => void;
  onAccountsChanged: () => void;
  onAddAccount: () => void;
  onClose: () => void;
}

export function SettingsView({ settings, accounts, onChange, onAccountsChanged, onAddAccount, onClose }: Props) {
  const id = useId();

  return (
    <section className="settings" aria-labelledby={`${id}-title`}>
      <header className="settings__head">
        <h1 id={`${id}-title`} className="settings__title">
          Settings
        </h1>
        <button type="button" className="icon-btn" aria-label="Close settings" onClick={onClose}>
          <Icon name="close" />
        </button>
      </header>

      <div className="settings__body">
        <fieldset className="settings__group">
          <legend className="settings__legend">Appearance</legend>
          {(['system', 'light', 'dark'] as ThemeSetting[]).map((theme) => (
            <label key={theme} className="choice">
              <input
                type="radio"
                name={`${id}-theme`}
                checked={settings.theme === theme}
                onChange={() => {
                  onChange({ ...settings, theme });
                }}
              />
              {theme === 'system' ? 'Match my system' : theme === 'light' ? 'Light' : 'Dark'}
            </label>
          ))}
        </fieldset>

        <fieldset className="settings__group">
          <legend className="settings__legend">Images in email</legend>
          <label className="choice">
            <input
              type="radio"
              name={`${id}-images`}
              checked={settings.remoteImages === 'block'}
              onChange={() => {
                onChange({ ...settings, remoteImages: 'block' });
              }}
            />
            Ask before loading images from the internet (recommended; stops senders tracking when you read)
          </label>
          <label className="choice">
            <input
              type="radio"
              name={`${id}-images`}
              checked={settings.remoteImages === 'allow'}
              onChange={() => {
                onChange({ ...settings, remoteImages: 'allow' });
              }}
            />
            Always load images
          </label>
        </fieldset>

        <fieldset className="settings__group">
          <legend className="settings__legend">Notifications</legend>
          <label className="choice">
            <input
              type="checkbox"
              checked={settings.notifications}
              onChange={(e) => {
                onChange({ ...settings, notifications: e.target.checked });
              }}
            />
            Show a desktop notification when new mail arrives
          </label>
          <div className="choice-row">
            <label className="choice">
              <input
                type="checkbox"
                checked={settings.sound}
                disabled={!settings.notifications}
                onChange={(e) => {
                  onChange({ ...settings, sound: e.target.checked });
                }}
              />
              Play a sound with new mail
            </label>
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => {
                playChime({ force: true });
              }}
            >
              Play sound
            </button>
          </div>
          <p className="form-hint">
            If nothing appears, check that notifications are allowed for Mocky Mail in your system settings.
          </p>
        </fieldset>

        <section className="settings__group" aria-labelledby={`${id}-accounts`}>
          <h2 id={`${id}-accounts`} className="settings__legend">
            Accounts
          </h2>
          <ul className="account-list">
            {accounts.map((account) => (
              <AccountRow key={account.id} account={account} onChanged={onAccountsChanged} />
            ))}
          </ul>
          <button type="button" className="btn btn--secondary" onClick={onAddAccount}>
            <Icon name="plus" />
            Add account
          </button>
        </section>
      </div>
    </section>
  );
}

function AccountRow({ account, onChanged }: { account: Account; onChanged: () => void }) {
  const api = useMailApi();
  const id = useId();
  const passwordRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState(account.displayName);
  const [confirming, setConfirming] = useState(false);
  const [changingPassword, setChangingPassword] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = (work: Promise<unknown>, success: string) => {
    setBusy(true);
    setError(null);
    setDone(null);
    work.then(
      () => {
        setDone(success);
        setBusy(false);
        onChanged();
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(false);
      },
    );
  };

  const rename = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || trimmed === account.displayName) return;
    run(api.updateAccount(account.id, { displayName: trimmed }), 'Name saved.');
  };

  const updatePassword = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const field = passwordRef.current;
    const password = field?.value ?? '';
    if (field) field.value = '';
    if (!password) return;
    setChangingPassword(false);
    run(api.updateAccount(account.id, { password }), 'Password updated.');
  };

  const copy = error ? accountErrorCopy(error, account.email) : null;
  const authLabel = account.auth === 'password' ? 'Password' : account.auth === 'google' ? 'Google sign-in' : 'Microsoft sign-in';

  return (
    <li className="account-row">
      <div className="account-row__head">
        <span className="dot" style={{ background: account.color }} aria-hidden="true" />
        <div className="account-row__who">
          <p className="account-row__email">{account.email}</p>
          <p className="muted small">
            {authLabel}, {account.imap.host}
          </p>
        </div>
        <button
          type="button"
          className="btn btn--danger-link"
          disabled={busy}
          onClick={() => {
            setConfirming(true);
          }}
        >
          Remove
        </button>
      </div>

      <form className="inline-form" onSubmit={rename}>
        <label className="form-field" htmlFor={`${id}-name`}>
          <span className="form-field__label">Name shown to recipients</span>
          <input
            id={`${id}-name`}
            className="field"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </label>
        <button type="submit" className="btn btn--secondary" disabled={busy || name.trim() === account.displayName}>
          Save name
        </button>
      </form>

      {account.auth === 'password' &&
        (changingPassword ? (
          <form className="inline-form" onSubmit={updatePassword}>
            <div className="form-field">
              <label className="form-field__label" htmlFor={`${id}-password`}>
                New password or app password
              </label>
              <PasswordField id={`${id}-password`} inputRef={passwordRef} autoComplete="new-password" />
            </div>
            <button type="submit" className="btn btn--primary" disabled={busy}>
              Update password
            </button>
            <button
              type="button"
              className="btn btn--link"
              onClick={() => {
                setChangingPassword(false);
              }}
            >
              Cancel
            </button>
          </form>
        ) : (
          <button
            type="button"
            className="btn btn--link"
            onClick={() => {
              setChangingPassword(true);
            }}
          >
            Change password
          </button>
        ))}

      {copy && (
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{copy.title}</p>
          <p>{copy.message}</p>
          {copy.hint && <p className="notice__hint">{copy.hint}</p>}
        </div>
      )}
      {done && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}

      {confirming && (
        <ConfirmDialog
          title={`Remove ${account.email}?`}
          description="Mail saved on this computer and the stored password are deleted. Your mail on the server is not touched; you can add the account again later."
          confirmLabel="Remove account"
          danger
          onCancel={() => {
            setConfirming(false);
          }}
          onConfirm={() => {
            setConfirming(false);
            run(api.removeAccount(account.id), 'Account removed.');
          }}
        />
      )}
    </li>
  );
}
