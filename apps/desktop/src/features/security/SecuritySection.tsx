import { useEffect, useId, useRef, useState, type ReactNode, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, SecurityKey, SecurityKind, Settings } from '../../api/types';
import { nowSeconds } from '../../lib/format';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog, Modal } from '../../ui/Modal';
import { PasswordField } from '../../ui/PasswordField';
import { expiryOf, formatFingerprint, keptBy, kindLabel, kindName, kindNoun, sourceLabel } from './securityText';
import { TrustKeyDialog } from './TrustKeyDialog';

interface Props {
  settings: Settings;
  onChange: (next: Settings) => void;
  accounts: readonly Account[];
  /** Opened from a message that needs a key: scroll here and focus the heading. */
  focus?: boolean;
  /** Unix seconds, for expiry. */
  now?: number;
}

/** "S/MIME certificate for you@work.example (expired)": a key, told apart from the others. */
function describe(key: SecurityKey, now: number): string {
  const who = key.isOwn ? (key.addresses[0] ?? key.owner) : key.owner;
  return `${kindLabel(key.kind)} for ${who}${expiryOf(key, now).expired ? ' (expired)' : ''}`;
}

const KINDS: readonly SecurityKind[] = ['smime', 'openpgp'];

/**
 * Settings › Security: your certificates and keys (import, export, remove),
 * which one each account signs with, whether new mail is signed and
 * encrypted by default, and the keys collected from other people.
 */
export function SecuritySection({ settings, onChange, accounts, focus = false, now = nowSeconds() }: Props) {
  const api = useMailApi();
  const id = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const [keys, setKeys] = useState<SecurityKey[] | null>(null);
  /** Bumped when keys come or go, so each account's choice is read again. */
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [removing, setRemoving] = useState<SecurityKey | null>(null);
  const [trusting, setTrusting] = useState<SecurityKey | null>(null);
  const [trustError, setTrustError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api.listKeys().then(
      (list) => {
        if (live) setKeys(list);
      },
      (e: unknown) => {
        if (!live) return;
        setError(toAppError(e));
        setKeys([]);
      },
    );
    return () => {
      live = false;
    };
  }, [api]);

  useEffect(() => {
    if (!focus) return;
    const el = heading.current;
    // scrollIntoView is missing in some test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    el?.scrollIntoView?.({ block: 'start' });
    el?.focus();
  }, [focus]);

  const changed = (list: SecurityKey[], message: string | null) => {
    setKeys(list);
    setRevision((r) => r + 1);
    setDone(message);
  };

  const failed = (e: unknown) => {
    setError(toAppError(e));
  };

  const clear = () => {
    setError(null);
    setDone(null);
  };

  const exportKey = (key: SecurityKey) => {
    clear();
    api.exportPublicKey(key.id).then((path) => {
      if (path !== null) setDone(`Saved: ${path}`);
    }, failed);
  };

  const remove = (key: SecurityKey) => {
    clear();
    setRemoving(null);
    api.removeKey(key.id).then((list) => {
      changed(list, `Removed the ${kindLabel(key.kind)} for ${key.isOwn ? (key.addresses[0] ?? key.owner) : key.owner}.`);
    }, failed);
  };

  const setTrusted = (key: SecurityKey, trusted: boolean) => {
    clear();
    setTrustError(null);
    api.setKeyTrusted(key.id, trusted).then(
      (list) => {
        setTrusting(null);
        changed(list, trusted ? `You trust ${key.owner}’s key.` : `You no longer trust ${key.owner}’s key.`);
      },
      (e: unknown) => {
        if (trusted) setTrustError(toAppError(e).message);
        else failed(e);
      },
    );
  };

  /** Remove, or where to remove it: keys in a system store are looked after there. */
  const removeControl = (key: SecurityKey) =>
    key.source === 'mocky' ? (
      <button
        type="button"
        className="btn btn--danger-link"
        aria-label={`Remove ${describe(key, now)}`}
        onClick={() => {
          clear();
          setRemoving(key);
        }}
      >
        Remove
      </button>
    ) : (
      <span className="muted small">Kept by {keptBy(key.source)}; remove it there.</span>
    );

  const own = keys?.filter((k) => k.isOwn) ?? [];
  const others = keys?.filter((k) => !k.isOwn) ?? [];

  return (
    <section className="settings__group settings__group--wide security" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} ref={heading} tabIndex={-1} className="settings__legend">
        Security
      </h2>
      <p className="form-hint">
        A signature lets people check that a message really came from you and wasn’t changed. Encrypted mail can only be
        read by the people it’s sent to. Both need a certificate (S/MIME) or a key (OpenPGP). Search doesn’t look
        inside encrypted messages.
      </p>

      <label className="choice">
        <input
          type="checkbox"
          checked={settings.signByDefault}
          onChange={(e) => {
            onChange({ ...settings, signByDefault: e.target.checked });
          }}
        />
        Sign new messages by default
      </label>
      <label className="choice">
        <input
          type="checkbox"
          checked={settings.encryptWhenPossible}
          onChange={(e) => {
            onChange({ ...settings, encryptWhenPossible: e.target.checked });
          }}
        />
        Encrypt when possible
      </label>
      <p className="form-hint">
        When every recipient’s certificate or key is known, the message is encrypted. You can still turn either off in a
        message’s Options.
      </p>

      <div className="settings__sub">
        <h3 id={`${id}-own`} className="form-field__label">
          Your keys and certificates
        </h3>
        {keys !== null && own.length === 0 && (
          <p className="form-hint">None yet. Import one to sign and encrypt your mail.</p>
        )}
        {own.length > 0 && (
          <ul className="key-list" aria-labelledby={`${id}-own`}>
            {own.map((key) => (
              <KeyRow key={key.id} keyInfo={key} now={now}>
                <button
                  type="button"
                  className="btn btn--secondary btn--small"
                  aria-label={`Export public key: ${describe(key, now)}`}
                  onClick={() => {
                    exportKey(key);
                  }}
                >
                  Export public key
                </button>
                {removeControl(key)}
              </KeyRow>
            ))}
          </ul>
        )}
        <button
          type="button"
          className="btn btn--secondary"
          onClick={() => {
            clear();
            setImporting(true);
          }}
        >
          <Icon name="plus" />
          Import key or certificate…
        </button>
      </div>

      {accounts.length > 0 && (
        <div className="settings__sub">
          <h3 className="form-field__label">Which to use for each account</h3>
          {accounts.map((account) => (
            <AccountKeys key={account.id} account={account} own={own} now={now} revision={revision} onError={failed} />
          ))}
        </div>
      )}

      <div className="settings__sub">
        <h3 id={`${id}-others`} className="form-field__label">
          Other people’s keys and certificates
        </h3>
        {keys !== null && others.length === 0 && (
          <p className="form-hint">None yet. They’re added when people send you signed mail, or when you import them.</p>
        )}
        {others.length > 0 && (
          <ul className="key-list" aria-labelledby={`${id}-others`}>
            {others.map((key) => (
              <KeyRow key={key.id} keyInfo={key} now={now} showFingerprint>
                {key.kind === 'openpgp' ? (
                  <label className="choice key-row__trust">
                    <input
                      type="checkbox"
                      checked={key.trusted}
                      onChange={(e) => {
                        if (e.target.checked) {
                          clear();
                          setTrustError(null);
                          setTrusting(key);
                        } else setTrusted(key, false);
                      }}
                    />
                    I’ve checked this key belongs to {key.owner}
                  </label>
                ) : (
                  <span className="muted small key-row__trust">Certificates are vouched for by whoever issued them.</span>
                )}
                {removeControl(key)}
              </KeyRow>
            ))}
          </ul>
        )}
      </div>

      {done && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}

      {importing && (
        <ImportDialog
          onClose={() => {
            setImporting(false);
          }}
          onImported={(list) => {
            const before = new Set((keys ?? []).map((k) => k.id));
            const added = list.filter((k) => !before.has(k.id));
            setImporting(false);
            changed(
              list,
              added.length === 0
                ? 'That key or certificate was already here; it has been updated.'
                : `Added: ${added.map((k) => describe(k, now)).join(', ')}.`,
            );
          }}
        />
      )}

      {removing && (
        <ConfirmDialog
          title={`Remove this ${kindNoun(removing.kind)}?`}
          description={
            removing.isOwn
              ? `Mail that was encrypted to it can’t be opened on this device any more, including messages you already have, and you can’t sign with it. Only remove it if you have a copy of the file somewhere safe, or are sure you won’t need it.`
              : `You won’t be able to encrypt mail to ${removing.owner} or check their signatures until their ${kindNoun(removing.kind)} is added again.`
          }
          confirmLabel={`Remove ${kindNoun(removing.kind)}`}
          danger
          onCancel={() => {
            setRemoving(null);
          }}
          onConfirm={() => {
            remove(removing);
          }}
        />
      )}

      {trusting && (
        <TrustKeyDialog
          owner={trusting.addresses[0] ? `${trusting.owner} <${trusting.addresses[0]}>` : trusting.owner}
          fingerprint={trusting.fingerprint}
          error={trustError}
          onConfirm={() => {
            setTrusted(trusting, true);
          }}
          onCancel={() => {
            setTrusting(null);
          }}
        />
      )}
    </section>
  );
}

function KeyRow({
  keyInfo: key,
  now,
  showFingerprint = false,
  children,
}: {
  keyInfo: SecurityKey;
  now: number;
  showFingerprint?: boolean;
  children: ReactNode;
}) {
  const expiry = expiryOf(key, now);
  return (
    <li className="key-row" aria-label={describe(key, now)}>
      <div className="key-row__head">
        <Icon name="key" />
        <div className="key-row__who">
          <p className="key-row__owner">
            {key.owner} <span className="muted">· {kindLabel(key.kind)}</span>
          </p>
          <p className="muted small key-row__addresses">For {key.addresses.join(', ')}</p>
          {showFingerprint && <p className="fingerprint small">{formatFingerprint(key.fingerprint)}</p>}
          <p className="small">
            <span className={expiry.expired ? 'key-row__expiry key-row__expiry--expired' : 'key-row__expiry muted'}>
              {expiry.text}
            </span>
            <span className="muted"> · Kept in {sourceLabel(key.source)}</span>
          </p>
        </div>
      </div>
      <div className="key-row__actions">{children}</div>
    </li>
  );
}

/** One account: the certificate and key it signs with, and which kind comes first. */
function AccountKeys({
  account,
  own,
  now,
  revision,
  onError,
}: {
  account: Account;
  own: readonly SecurityKey[];
  now: number;
  revision: number;
  onError: (e: unknown) => void;
}) {
  const api = useMailApi();
  const [chosen, setChosen] = useState<{ smime: string | null; openpgp: string | null; preferred: SecurityKind } | null>(
    null,
  );

  useEffect(() => {
    let live = true;
    api.getAccountKeys(account.id).then(
      (next) => {
        if (live) setChosen(next);
      },
      (e: unknown) => {
        if (live) onError(e);
      },
    );
    return () => {
      live = false;
    };
    // onError only reports; reloading when it changes identity would loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, account.id, revision]);

  if (!chosen) return null;
  const email = account.email.toLowerCase();
  const fits = (kind: SecurityKind) =>
    own.filter(
      (k) => k.kind === kind && k.source === 'mocky' && k.canSign && k.addresses.some((a) => a.toLowerCase() === email),
    );
  // Held by Windows, the keychain, a smart card or GnuPG: Mocky Mail can't sign with those yet.
  const elsewhere = own.filter((k) => k.source !== 'mocky' && k.addresses.some((a) => a.toLowerCase() === email));

  const pick = (kind: SecurityKind, keyId: string | null) => {
    const previous = chosen;
    setChosen({ ...chosen, [kind]: keyId });
    api.setAccountKey(account.id, kind, keyId).catch((e: unknown) => {
      setChosen(previous);
      onError(e);
    });
  };

  const prefer = (kind: SecurityKind) => {
    const previous = chosen;
    setChosen({ ...chosen, preferred: kind });
    api.setPreferredKind(account.id, kind).catch((e: unknown) => {
      setChosen(previous);
      onError(e);
    });
  };

  return (
    <fieldset className="key-account">
      <legend className="key-account__email">
        {account.displayName ? `${account.displayName} (${account.email})` : `Account: ${account.email}`}
      </legend>
      {KINDS.map((kind) => {
        const options = fits(kind);
        const current = chosen[kind];
        // Keep showing a choice that no longer fits (it was made before the address changed).
        const stray = current !== null && !options.some((k) => k.id === current) ? own.find((k) => k.id === current) : undefined;
        return (
          <label key={kind} className="form-field">
            <span className="form-field__label">{kindLabel(kind)}</span>
            <select
              className="field"
              value={current ?? ''}
              onChange={(e) => {
                pick(kind, e.target.value || null);
              }}
            >
              <option value="">None</option>
              {[...options, ...(stray ? [stray] : [])].map((k) => (
                <option key={k.id} value={k.id}>
                  {`${k.owner} · ${expiryOf(k, now).text} · …${formatFingerprint(k.fingerprint).slice(-9)}`}
                </option>
              ))}
            </select>
          </label>
        );
      })}
      {fits('smime').length === 0 && fits('openpgp').length === 0 && (
        <p className="form-hint">No certificate or key for {account.email} yet. Import one above.</p>
      )}
      {elsewhere.length > 0 && (
        <p className="form-hint">
          {elsewhere.map((k) => `${describe(k, now)} is kept by ${keptBy(k.source)}`).join('; ')}, so it can’t be
          chosen here. To sign or open encrypted mail with it, export it from there (with its private key) and import
          the file above.
        </p>
      )}
      <label className="form-field">
        <span className="form-field__label">
          Use first
        </span>
        <select
          className="field"
          value={chosen.preferred}
          onChange={(e) => {
            const kind = KINDS.find((k) => k === e.target.value);
            if (kind) prefer(kind);
          }}
        >
          {KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {kindName(kind)}
            </option>
          ))}
        </select>
      </label>
    </fieldset>
  );
}

/**
 * Asks for the file's password (if it has one) before the file is picked.
 * The password stays in the field until Choose file, then the field is
 * emptied: the interface never keeps it.
 */
function ImportDialog({ onClose, onImported }: { onClose: () => void; onImported: (keys: SecurityKey[]) => void }) {
  const api = useMailApi();
  const id = useId();
  const field = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const submit = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    const input = field.current;
    const password = input?.value ?? '';
    if (input) input.value = '';
    setBusy(true);
    setError(null);
    api.importKey(password === '' ? null : password).then(
      (list) => {
        setBusy(false);
        if (list === null) onClose();
        else onImported(list);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
        field.current?.focus();
      },
    );
  };

  return (
    <Modal
      title="Import a key or certificate"
      description="Next, choose the file: a certificate (.p12, .pfx, .cer or .pem) or a key (.asc or .gpg)."
      onClose={onClose}
      size="small"
    >
      <form onSubmit={submit}>
        <div className="form-field">
          <label className="form-field__label" htmlFor={`${id}-password`}>
            The file’s password, if it has one
          </label>
          <PasswordField
            id={`${id}-password`}
            inputRef={field}
            autoComplete="off"
            describedBy={`${id}-hint`}
            invalid={error?.kind === 'auth'}
          />
          <p id={`${id}-hint`} className="form-hint">
            It’s used once to open the file and isn’t kept. Leave it empty for a file without one.
          </p>
        </div>
        {error && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {busy ? 'Opening' : 'Choose file…'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
