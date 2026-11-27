import { useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Category, Folder, Settings, ThemeSetting, MarkReadSetting } from '../../api/types';
import type { Notice } from '../../app/useToasts';
import { RulesSection } from '../rules/RulesSection';
import { TrustedSenders } from './TrustedSenders';
import { CategoryManager } from '../categories/CategoryManager';
import { ConfirmDialog } from '../../ui/Modal';
import { Icon } from '../../ui/Icon';
import { PasswordField } from '../../ui/PasswordField';
import { reminderText } from '../calendar/calendarTime';
import { OutOfOffice } from './OutOfOffice';
import { playChime } from '../notifications/chime';
import { accountErrorCopy } from '../accounts/providers';
import { SignatureEditor } from './SignatureEditor';
import { ImportExportSection } from '../files/ImportExportSection';

const MARK_READ: readonly [MarkReadSetting, string][] = [
  ['open', 'Mark as read as soon as I open a message'],
  ['delay', 'Mark as read after a few seconds open (a quick peek stays unread)'],
  ['manual', 'Only when I choose Mark as read'],
];

/** Same choices as the backend accepts (settings.rs). */
const CHECK_EVERY = [1, 2, 5, 10, 15, 30] as const;
const READ_DELAY = [1, 3, 5, 10] as const;

/** Same choices as the backend accepts (UNDO_SEND_CHOICES in settings.rs). */
const UNDO_SEND = [0, 5, 10, 20, 30] as const;

const READ_RECEIPTS: readonly [Settings['readReceipts'], string][] = [
  ['ask', 'Ask each time'],
  ['always', 'Always send'],
  ['never', 'Never send'],
];

const READING_PANE: readonly [Settings['readingPane'], string][] = [
  ['right', 'Right of the list'],
  ['bottom', 'Below the list'],
  ['off', 'Off: open messages over the list'],
];

/** How much mail is kept in full on this device; null keeps everything. */
export const SYNC_WINDOW: readonly [number | null, string][] = [
  [1, '1 month'],
  [3, '3 months'],
  [6, '6 months'],
  [12, '12 months'],
  [24, '24 months'],
  [null, 'All mail'],
];

interface Props {
  settings: Settings;
  accounts: readonly Account[];
  onChange: (next: Settings) => void;
  onAccountsChanged: () => void;
  onAddAccount: () => void;
  onClose: () => void;
  /** Colour categories, managed here. */
  categories?: readonly Category[];
  onCategoriesChanged?: (categories: Category[]) => void;
  /** Phones: no reading pane choice (a message always fills the screen). */
  phone?: boolean;
  /** Every account's folders, for rules. */
  folders?: Readonly<Record<string, Folder[]>>;
  /** A toast (rules run now say what they did). */
  onNotice?: (notice: Notice) => void;
}

export function SettingsView({
  settings,
  accounts,
  onChange,
  onAccountsChanged,
  onAddAccount,
  onClose,
  categories,
  onCategoriesChanged,
  phone = false,
  folders,
  onNotice,
}: Props) {
  const id = useId();
  const api = useMailApi();
  const [orderError, setOrderError] = useState<AppError | null>(null);

  /** Move an account one place up (-1) or down (1) in the sidebar. */
  const move = (accountId: string, by: -1 | 1) => {
    const ids = accounts.map((a) => a.id);
    const from = ids.indexOf(accountId);
    const to = from + by;
    if (from < 0 || to < 0 || to >= ids.length) return;
    ids.splice(from, 1);
    ids.splice(to, 0, accountId);
    setOrderError(null);
    api.reorderAccounts(ids).then(onAccountsChanged, (e: unknown) => {
      setOrderError(toAppError(e));
    });
  };

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
        <div className="settings__columns">
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
            {!phone && (
              <fieldset className="settings__sub">
                <legend className="form-field__label">Reading pane</legend>
                {READING_PANE.map(([value, label]) => (
                  <label key={value} className="choice">
                    <input
                      type="radio"
                      name={`${id}-pane`}
                      checked={settings.readingPane === value}
                      onChange={() => {
                        onChange({ ...settings, readingPane: value });
                      }}
                    />
                    {label}
                  </label>
                ))}
              </fieldset>
            )}
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
            <TrustedSenders />
          </fieldset>

          <fieldset className="settings__group">
            <legend className="settings__legend">Reading</legend>
            {MARK_READ.map(([value, label]) => (
              <label key={value} className="choice">
                <input
                  type="radio"
                  name={`${id}-mark-read`}
                  checked={settings.markRead === value}
                  onChange={() => {
                    onChange({ ...settings, markRead: value });
                  }}
                />
                {label}
              </label>
            ))}
            <label className="choice">
              <input
                type="checkbox"
                checked={settings.threads}
                onChange={(e) => {
                  onChange({ ...settings, threads: e.target.checked });
                }}
              />
              Group replies into conversations
            </label>
            <label className="choice">
              <input
                type="checkbox"
                checked={settings.focusedInbox}
                onChange={(e) => {
                  onChange({ ...settings, focusedInbox: e.target.checked });
                }}
              />
              Focused Inbox: put newsletters and notifications in a separate Other tab
            </label>
            {settings.markRead === 'delay' && (
              <label className="form-field">
                <span className="form-field__label">Mark as read after</span>
                <select
                  className="field"
                  value={settings.markReadDelaySecs}
                  onChange={(e) => {
                    onChange({
                      ...settings,
                      markReadDelaySecs: Number(e.target.value),
                    });
                  }}
                >
                  {READ_DELAY.map((secs) => (
                    <option key={secs} value={secs}>
                      {secs === 1 ? '1 second' : `${String(secs)} seconds`}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <label className="form-field">
              <span className="form-field__label">Read receipts</span>
              <select
                className="field"
                value={settings.readReceipts}
                onChange={(e) => {
                  const value = READ_RECEIPTS.find(([v]) => v === e.target.value)?.[0];
                  if (value) onChange({ ...settings, readReceipts: value });
                }}
              >
                {READ_RECEIPTS.map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <p className="form-hint">Read receipts: what to do when a message you open asks to know that you read it.</p>
          </fieldset>

          <fieldset className="settings__group">
            <legend className="settings__legend">Checking for mail</legend>
            <label className="form-field">
              <span className="form-field__label">Check for new mail every</span>
              <select
                className="field"
                value={settings.checkEveryMins}
                onChange={(e) => {
                  onChange({
                    ...settings,
                    checkEveryMins: Number(e.target.value),
                  });
                }}
              >
                {CHECK_EVERY.map((mins) => (
                  <option key={mins} value={mins}>
                    {mins === 1 ? '1 minute' : `${String(mins)} minutes`}
                  </option>
                ))}
              </select>
            </label>
            <p className="form-hint">
              Most servers tell Mocky Mail about new mail straight away; this is for those that can't. Other folders are
              checked at least every 10 minutes.
            </p>
            <label className="choice">
              <input
                type="checkbox"
                checked={settings.workOffline}
                onChange={(e) => {
                  onChange({ ...settings, workOffline: e.target.checked });
                }}
              />
              Work offline: don't check for mail or send anything until I switch this off
            </label>
            {settings.workOffline && (
              <p className="form-hint" role="status">
                Working offline. Messages you send wait in the Outbox.
              </p>
            )}
          </fieldset>

          <fieldset className="settings__group">
            <legend className="settings__legend">Sending</legend>
            <label className="form-field">
              <span className="form-field__label">Undo send</span>
              <select
                className="field"
                value={settings.undoSendSecs}
                onChange={(e) => {
                  onChange({
                    ...settings,
                    undoSendSecs: Number(e.target.value),
                  });
                }}
              >
                {UNDO_SEND.map((secs) => (
                  <option key={secs} value={secs}>
                    {secs === 0 ? 'Off: send at once' : `${String(secs)} seconds`}
                  </option>
                ))}
              </select>
            </label>
            <p className="form-hint">After you tap Send, you have this long to take a message back.</p>
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
              Show a notification when new mail arrives
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
            <label className="choice">
              <input
                type="checkbox"
                checked={settings.eventReminders}
                onChange={(e) => {
                  onChange({ ...settings, eventReminders: e.target.checked });
                }}
              />
              Remind me before calendar events
            </label>
            <label className="form-field">
              <span className="form-field__label">Reminder for new events</span>
              <select
                className="field"
                value={settings.defaultReminderMins}
                onChange={(e) => {
                  onChange({ ...settings, defaultReminderMins: Number(e.target.value) });
                }}
              >
                {[-1, 0, 5, 10, 15, 30, 60, 120, 1440].map((m) => (
                  <option key={m} value={m}>
                    {reminderText(m)}
                  </option>
                ))}
              </select>
            </label>
            <p className="form-hint">
              If nothing appears, check that notifications are allowed for Mocky Mail in your system settings.
            </p>
          </fieldset>

          {settings.device.tray && (
            <fieldset className="settings__group">
              <legend className="settings__legend">On this computer</legend>
              <label className="choice">
                <input
                  type="checkbox"
                  checked={settings.closeToTray}
                  onChange={(e) => {
                    onChange({ ...settings, closeToTray: e.target.checked });
                  }}
                />
                Keep running in the tray when the window is closed
              </label>
              {settings.device.startAtLogin && (
                <label className="choice">
                  <input
                    type="checkbox"
                    checked={settings.startAtLogin}
                    onChange={(e) => {
                      onChange({ ...settings, startAtLogin: e.target.checked });
                    }}
                  />
                  Open Mocky Mail when I sign in to this computer
                </label>
              )}
              <p className="form-hint">
                In the tray, Mocky Mail keeps checking for mail and showing notifications. Quit it from the tray icon.
                {settings.device.startAtLogin ? ' At sign-in it opens straight into the tray.' : ''}
              </p>
            </fieldset>
          )}

          {accounts.length > 0 && <OutOfOffice accounts={accounts} />}

          {onCategoriesChanged && <CategoryManager categories={categories ?? []} onChange={onCategoriesChanged} />}

          {folders && accounts.length > 0 && (
            <RulesSection
              accounts={accounts}
              folders={folders}
              categories={categories ?? []}
              phone={phone}
              {...(onNotice ? { onNotice } : {})}
            />
          )}

          {folders && accounts.length > 0 && <ImportExportSection accounts={accounts} folders={folders} />}

          <section className="settings__group settings__group--wide" aria-labelledby={`${id}-accounts`}>
            <h2 id={`${id}-accounts`} className="settings__legend">
              Accounts
            </h2>
            <ul className="account-list">
              {accounts.map((account, index) => (
                <AccountRow
                  key={account.id}
                  account={account}
                  onChanged={onAccountsChanged}
                  {...(accounts.length > 1
                    ? {
                        order: {
                          first: index === 0,
                          last: index === accounts.length - 1,
                          onMove: (by: -1 | 1) => {
                            move(account.id, by);
                          },
                        },
                      }
                    : {})}
                />
              ))}
            </ul>
            {orderError && (
              <p className="form-error" role="alert">
                {orderError.message}
              </p>
            )}
            <button type="button" className="btn btn--secondary" onClick={onAddAccount}>
              <Icon name="plus" />
              Add account
            </button>
          </section>
        </div>
      </div>
    </section>
  );
}

interface AccountOrder {
  first: boolean;
  last: boolean;
  onMove: (by: -1 | 1) => void;
}

function AccountRow({ account, onChanged, order }: { account: Account; onChanged: () => void; order?: AccountOrder }) {
  const api = useMailApi();
  const id = useId();
  const passwordRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState(account.displayName);
  const [confirming, setConfirming] = useState(false);
  const [changingPassword, setChangingPassword] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The provider's sign-in page is open in the browser. */
  const [reconnecting, setReconnecting] = useState(false);

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

  const reconnect = () => {
    setReconnecting(true);
    run(
      api.reconnectAccount(account.id).finally(() => {
        setReconnecting(false);
      }),
      'Signed in again.',
    );
  };

  const copy = error ? accountErrorCopy(error, account.email) : null;
  const authLabel =
    account.auth === 'password' ? 'Password' : account.auth === 'google' ? 'Google sign-in' : 'Microsoft sign-in';

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
        {order && (
          <span className="account-row__order">
            <button
              type="button"
              className="icon-btn"
              aria-label={`Move ${account.email} up`}
              title="Move up"
              disabled={order.first}
              onClick={() => {
                order.onMove(-1);
              }}
            >
              <span className="flip" aria-hidden="true">
                <Icon name="chevron" />
              </span>
            </button>
            <button
              type="button"
              className="icon-btn"
              aria-label={`Move ${account.email} down`}
              title="Move down"
              disabled={order.last}
              onClick={() => {
                order.onMove(1);
              }}
            >
              <Icon name="chevron" />
            </button>
          </span>
        )}
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

      <label className="form-field account-row__sync">
        <span className="form-field__label">Keep mail on this device</span>
        <select
          className="field"
          value={account.syncMonths === null ? 'all' : String(account.syncMonths)}
          disabled={busy}
          onChange={(e) => {
            const months = e.target.value === 'all' ? null : Number(e.target.value);
            run(
              api.updateAccount(account.id, { syncMonths: months }),
              months === null ? 'All mail is kept on this device.' : `Mail from the last ${SYNC_WINDOW.find(([m]) => m === months)?.[1] ?? ''} is kept on this device.`,
            );
          }}
        >
          {SYNC_WINDOW.map(([months, label]) => (
            <option key={label} value={months === null ? 'all' : String(months)}>
              {label}
            </option>
          ))}
        </select>
      </label>
      <p className="form-hint">Older mail keeps its sender, subject and preview, and opens from the server when you need it.</p>

      <SignatureEditor accountId={account.id} />

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

      {account.auth !== 'password' && (
        // A Google or Microsoft sign-in can expire or be withdrawn; this opens
        // the provider's sign-in again and keeps the account and its mail.
        <button
          type="button"
          className="btn btn--link"
          disabled={busy}
          aria-busy={reconnecting}
          onClick={reconnect}
        >
          {reconnecting ? 'Waiting for sign-in' : 'Sign in again'}
        </button>
      )}

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
          description="Mail saved on this device and the stored password are deleted. Your mail on the server is not touched; you can add the account again later."
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
