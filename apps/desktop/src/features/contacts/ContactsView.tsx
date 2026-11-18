import { useEffect, useId, useMemo, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AddressBookStatus, AppError, Contact, ContactInput, Mailbox } from '../../api/types';
import { useDebounced } from '../../lib/useDebounced';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog } from '../../ui/Modal';

interface Props {
  accounts: readonly Account[];
  /** Phone layout: list or one contact fills the screen. */
  phone: boolean;
  onClose: () => void;
  /** Start a new message to this address. */
  onWrite: (name: string, address: string) => void;
  /** Show mail from and to this address. */
  onFindMail: (address: string) => void;
}

type Pane = { kind: 'none' } | { kind: 'view'; id: string } | { kind: 'edit'; input: ContactInput };

const EMPTY: Omit<ContactInput, 'accountId'> = {
  name: '',
  emails: [''],
  phones: [''],
  organization: '',
  title: '',
  notes: '',
};

/**
 * The address book: contacts saved here or synced with each account's mail
 * server (CardDAV), plus people from mail who aren't saved yet. Search,
 * a contact card, add, edit and delete, and one tap to write to someone or
 * see their mail.
 */
export function ContactsView({ accounts, phone, onClose, onWrite, onFindMail }: Props) {
  const api = useMailApi();
  const id = useId();
  const [query, setQuery] = useState('');
  const search = useDebounced(query, 200);
  const [contacts, setContacts] = useState<Contact[] | null>(null);
  const [fromMail, setFromMail] = useState<Mailbox[]>([]);
  const [books, setBooks] = useState<AddressBookStatus[]>([]);
  const [revision, setRevision] = useState(0);
  const [pane, setPane] = useState<Pane>({ kind: 'none' });
  const [error, setError] = useState<AppError | null>(null);
  // Syncing starts as soon as the screen opens.
  const [syncing, setSyncing] = useState(true);

  useEffect(() => {
    let live = true;
    api.listContacts(search).then(
      (list) => {
        if (live) setContacts(list);
      },
      (e: unknown) => {
        if (live) setError(toAppError(e));
      },
    );
    if (search.trim()) {
      api.suggestRecipients(search).then(
        (people) => {
          if (live) setFromMail(people);
        },
        () => undefined,
      );
    }
    return () => {
      live = false;
    };
  }, [api, search, revision]);

  // Sync when opened; refresh whenever a sync finishes.
  useEffect(() => {
    const off = api.on('mail://contacts-changed', ({ books: next }) => {
      setBooks(next);
      setRevision((r) => r + 1);
    });
    api.syncContacts().then(
      (next) => {
        setBooks(next);
        setSyncing(false);
        setRevision((r) => r + 1);
      },
      () => {
        setSyncing(false);
      },
    );
    return off;
  }, [api]);

  const saved = useMemo(
    () => new Set((contacts ?? []).flatMap((c) => c.emails.map((e) => e.toLowerCase()))),
    [contacts],
  );
  const unsaved = (search.trim() ? fromMail : []).filter((m) => !saved.has(m.address.toLowerCase()));
  const syncedBooks = accounts.filter((a) => books.some((b) => b.accountId === a.id && b.state === 'synced'));

  const startNew = (prefill: Partial<ContactInput> = {}) => {
    setError(null);
    setPane({
      kind: 'edit',
      input: { ...EMPTY, accountId: syncedBooks[0]?.id ?? null, ...prefill },
    });
  };

  const selected = pane.kind === 'view' ? (contacts ?? []).find((c) => c.id === pane.id) : undefined;
  const showList = !phone || pane.kind === 'none';
  const showPane = !phone || pane.kind !== 'none';

  return (
    <section className="contacts" aria-labelledby={`${id}-title`}>
      <header className="contacts__head">
        <h1 id={`${id}-title`} className="contacts__title">
          Contacts
        </h1>
        <button
          type="button"
          className="icon-btn"
          aria-label="Sync contacts"
          title="Sync contacts"
          disabled={syncing}
          onClick={() => {
            setSyncing(true);
            api.syncContacts().then(
              (next) => {
                setBooks(next);
                setSyncing(false);
                setRevision((r) => r + 1);
              },
              (e: unknown) => {
                setError(toAppError(e));
                setSyncing(false);
              },
            );
          }}
        >
          <Icon name="sync" />
        </button>
        <button
          type="button"
          className="btn btn--primary contacts__new"
          onClick={() => {
            startNew();
          }}
        >
          <Icon name="plus" />
          New contact
        </button>
        <button type="button" className="icon-btn" aria-label="Close contacts" onClick={onClose}>
          <Icon name="close" />
        </button>
      </header>

      <div className="contacts__body">
        {showList && (
          <div className="contacts__list-pane">
            <label className="visually-hidden" htmlFor={`${id}-search`}>
              Search contacts
            </label>
            <input
              id={`${id}-search`}
              className="field contacts__search"
              type="search"
              placeholder="Search name, email, phone or company"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
              }}
            />
            <BookNotes accounts={accounts} books={books} />
            {contacts === null ? (
              <p className="muted contacts__empty" role="status">
                Loading contacts…
              </p>
            ) : (
              <>
                {contacts.length === 0 && unsaved.length === 0 && (
                  <p className="muted contacts__empty">
                    {search.trim() ? 'No contacts match.' : 'No contacts yet. Add one with New contact.'}
                  </p>
                )}
                {contacts.length > 0 && (
                  <ul className="contacts__list" aria-label="Contacts">
                    {contacts.map((c) => (
                      <li key={c.id}>
                        <button
                          type="button"
                          className={`contact-row${pane.kind === 'view' && pane.id === c.id ? ' contact-row--active' : ''}`}
                          aria-current={pane.kind === 'view' && pane.id === c.id ? 'true' : undefined}
                          onClick={() => {
                            setError(null);
                            setPane({ kind: 'view', id: c.id });
                          }}
                        >
                          <Avatar name={c.name} address={c.emails[0] ?? c.name} />
                          <span className="contact-row__text">
                            <span className="contact-row__name">{displayName(c)}</span>
                            <span className="muted small">{c.emails[0] ?? c.phones[0] ?? c.organization}</span>
                          </span>
                          {c.state !== 'synced' && (
                            <span className="contact-row__pending" title="Waiting to sync">
                              <Icon name="clock" size={14} />
                            </span>
                          )}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                {unsaved.length > 0 && (
                  <>
                    <h2 className="contacts__subhead">From your mail</h2>
                    <ul className="contacts__list" aria-label="People from your mail">
                      {unsaved.map((m) => (
                        <li key={m.address} className="contact-row contact-row--static">
                          <Avatar name={m.name ?? ''} address={m.address} />
                          <span className="contact-row__text">
                            <span className="contact-row__name">{m.name ?? m.address}</span>
                            <span className="muted small">{m.address}</span>
                          </span>
                          <button
                            type="button"
                            className="btn btn--secondary btn--small"
                            aria-label={`Save ${m.name ?? m.address} as a contact`}
                            onClick={() => {
                              startNew({
                                name: m.name ?? '',
                                emails: [m.address],
                              });
                            }}
                          >
                            Save
                          </button>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </>
            )}
          </div>
        )}

        {showPane && (
          <div className="contacts__pane">
            {error && (
              <p className="form-error" role="alert">
                {error.message}
              </p>
            )}
            {pane.kind === 'none' && !phone && (
              <p className="muted contacts__empty">Pick a contact to see their details.</p>
            )}
            {pane.kind === 'view' && selected && (
              <ContactCard
                contact={selected}
                accounts={accounts}
                phone={phone}
                onBack={() => {
                  setPane({ kind: 'none' });
                }}
                onEdit={() => {
                  setPane({
                    kind: 'edit',
                    input: {
                      ...selected,
                      emails: [...selected.emails, ''],
                      phones: [...selected.phones, ''],
                    },
                  });
                }}
                onDeleted={() => {
                  setPane({ kind: 'none' });
                  setRevision((r) => r + 1);
                }}
                onError={setError}
                onWrite={onWrite}
                onFindMail={onFindMail}
              />
            )}
            {pane.kind === 'edit' && (
              <ContactEditor
                key={pane.input.id ?? 'new'}
                initial={pane.input}
                choices={syncedBooks}
                onCancel={() => {
                  setPane(pane.input.id ? { kind: 'view', id: pane.input.id } : { kind: 'none' });
                }}
                onSaved={(contact) => {
                  setRevision((r) => r + 1);
                  setPane({ kind: 'view', id: contact.id });
                }}
              />
            )}
          </div>
        )}
      </div>
    </section>
  );
}

/** What to call a contact: name, else first email, else first phone. */
function displayName(c: Contact): string {
  if (c.name !== '') return c.name;
  return c.emails[0] ?? c.phones[0] ?? '';
}

/** Which accounts' contacts sync, and why others don't. */
function BookNotes({ accounts, books }: { accounts: readonly Account[]; books: AddressBookStatus[] }) {
  const problems = books.filter((b) => b.state === 'error');
  if (problems.length === 0) return null;
  return (
    <div className="contacts__notes" role="status">
      {problems.map((b) => (
        <p key={b.accountId} className="form-hint">
          {accounts.find((a) => a.id === b.accountId)?.email}: contacts didn't sync. {b.message}
        </p>
      ))}
    </div>
  );
}

function ContactCard({
  contact,
  accounts,
  phone,
  onBack,
  onEdit,
  onDeleted,
  onError,
  onWrite,
  onFindMail,
}: {
  contact: Contact;
  accounts: readonly Account[];
  phone: boolean;
  onBack: () => void;
  onEdit: () => void;
  onDeleted: () => void;
  onError: (e: AppError) => void;
  onWrite: (name: string, address: string) => void;
  onFindMail: (address: string) => void;
}) {
  const api = useMailApi();
  const [confirm, setConfirm] = useState(false);
  const book = contact.accountId ? accounts.find((a) => a.id === contact.accountId) : undefined;
  const where = book ? `Saved in ${book.email}'s address book` : 'Saved on this device only';
  const display = displayName(contact);
  const role = [contact.title, contact.organization].filter(Boolean).join(', ');

  return (
    <article className="contact-card" aria-label={display}>
      {phone && (
        <button type="button" className="icon-btn contact-card__back" aria-label="Back to contacts" onClick={onBack}>
          <Icon name="back" />
        </button>
      )}
      <div className="contact-card__head">
        <Avatar name={contact.name} address={contact.emails[0] ?? display} size={64} />
        <div>
          <h2 className="contact-card__name">{display}</h2>
          {role && <p className="muted">{role}</p>}
          <p className="muted small">
            {where}
            {contact.state !== 'synced' && ' · waiting to sync'}
          </p>
        </div>
      </div>

      <div className="contact-card__actions">
        {contact.emails[0] && (
          <>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                onWrite(contact.name, contact.emails[0] ?? '');
              }}
            >
              <Icon name="pen" />
              Write email
            </button>
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                onFindMail(contact.emails[0] ?? '');
              }}
            >
              <Icon name="search" />
              See emails
            </button>
          </>
        )}
        <button type="button" className="btn btn--secondary" onClick={onEdit}>
          Edit
        </button>
        <button
          type="button"
          className="btn btn--link btn--danger-text"
          onClick={() => {
            setConfirm(true);
          }}
        >
          Delete
        </button>
      </div>

      <dl className="contact-card__fields">
        {contact.emails.map((email, i) => (
          <div key={email} className="contact-card__field">
            <dt>{i === 0 ? 'Email' : 'Other email'}</dt>
            <dd>
              <button
                type="button"
                className="btn btn--link"
                aria-label={`Write to ${email}`}
                onClick={() => {
                  onWrite(contact.name, email);
                }}
              >
                {email}
              </button>
            </dd>
          </div>
        ))}
        {contact.phones.map((p, i) => (
          <div key={p} className="contact-card__field">
            <dt>{i === 0 ? 'Phone' : 'Other phone'}</dt>
            <dd>{p}</dd>
          </div>
        ))}
        {contact.notes && (
          <div className="contact-card__field">
            <dt>Notes</dt>
            <dd className="contact-card__notes">{contact.notes}</dd>
          </div>
        )}
      </dl>

      {confirm && (
        <ConfirmDialog
          title={`Delete ${display}?`}
          description={
            book
              ? `This also deletes the contact from ${book.email}'s address book on the server.`
              : 'This contact is only on this device.'
          }
          confirmLabel="Delete contact"
          danger
          onCancel={() => {
            setConfirm(false);
          }}
          onConfirm={() => {
            setConfirm(false);
            api.deleteContact(contact.id).then(onDeleted, (e: unknown) => {
              onError(toAppError(e));
            });
          }}
        />
      )}
    </article>
  );
}

function ContactEditor({
  initial,
  choices,
  onCancel,
  onSaved,
}: {
  initial: ContactInput;
  /** Accounts whose address book syncs; a new contact may go in one of them. */
  choices: readonly Account[];
  onCancel: () => void;
  onSaved: (contact: Contact) => void;
}) {
  const api = useMailApi();
  const id = useId();
  const [input, setInput] = useState<ContactInput>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const isNew = initial.id === undefined;

  const set = (patch: Partial<ContactInput>) => {
    setInput((i) => ({ ...i, ...patch }));
  };

  const listField = (field: 'emails' | 'phones', label: string, type: string) => (
    <fieldset className="contact-editor__list">
      <legend className="form-field__label">{label}</legend>
      {input[field].map((value, i) => (
        <input
          key={i}
          className="field"
          type={type}
          inputMode={type === 'tel' ? 'tel' : 'email'}
          aria-label={`${label} ${String(i + 1)}`}
          value={value}
          onChange={(e) => {
            const next = [...input[field]];
            next[i] = e.target.value;
            set({ [field]: next });
          }}
        />
      ))}
      <button
        type="button"
        className="btn btn--link"
        onClick={() => {
          set({ [field]: [...input[field], ''] });
        }}
      >
        Add another
      </button>
    </fieldset>
  );

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    api.saveContact(input).then(onSaved, (e: unknown) => {
      setError(toAppError(e));
      setBusy(false);
    });
  };

  return (
    <form className="contact-editor" aria-labelledby={`${id}-title`} noValidate onSubmit={submit}>
      <h2 id={`${id}-title`} className="contact-card__name">
        {isNew ? 'New contact' : 'Edit contact'}
      </h2>
      <label className="form-field">
        <span className="form-field__label">Name</span>
        <input
          className="field"
          value={input.name}
          autoComplete="off"
          onChange={(e) => {
            set({ name: e.target.value });
          }}
        />
      </label>
      {listField('emails', 'Email', 'email')}
      {listField('phones', 'Phone', 'tel')}
      <label className="form-field">
        <span className="form-field__label">Company</span>
        <input
          className="field"
          value={input.organization}
          onChange={(e) => {
            set({ organization: e.target.value });
          }}
        />
      </label>
      <label className="form-field">
        <span className="form-field__label">Job title</span>
        <input
          className="field"
          value={input.title}
          onChange={(e) => {
            set({ title: e.target.value });
          }}
        />
      </label>
      <label className="form-field">
        <span className="form-field__label">Notes</span>
        <textarea
          className="field contact-editor__notes"
          value={input.notes}
          onChange={(e) => {
            set({ notes: e.target.value });
          }}
        />
      </label>
      {isNew && (
        <label className="form-field">
          <span className="form-field__label">Save to</span>
          <select
            className="field"
            value={input.accountId ?? ''}
            onChange={(e) => {
              set({ accountId: e.target.value || null });
            }}
          >
            {choices.map((a) => (
              <option key={a.id} value={a.id}>
                {a.email} (synced with the mail server)
              </option>
            ))}
            <option value="">This device only</option>
          </select>
        </label>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onCancel} disabled={busy}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {busy ? 'Saving…' : 'Save contact'}
        </button>
      </div>
    </form>
  );
}
