import { useEffect, useId, useMemo, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AddressBookStatus, AppError, Category, Contact, ContactGroup, ContactInput, Mailbox } from '../../api/types';
import { ConnectCards } from '../connect/ConnectCard';
import { photoFromFile } from './photo';
import { useDebounced } from '../../lib/useDebounced';
import { Avatar } from '../../ui/Avatar';
import { Icon } from '../../ui/Icon';
import { MenuButton, MenuItem } from '../../ui/Menu';
import { ConfirmDialog, Modal } from '../../ui/Modal';
import { AddressBookPicker } from '../files/AddressBookPicker';
import { useFileTasks } from '../files/FileTasks';
import { exportContactsTask, importContactsTask } from '../files/tasks';

interface Props {
  accounts: readonly Account[];
  /** Phone layout: list or one contact fills the screen. */
  phone: boolean;
  onClose: () => void;
  /** Start a new message to this address. */
  onWrite: (name: string, address: string) => void;
  /** Show mail from and to this address. */
  onFindMail: (address: string) => void;
  /** Start a new message to several people (a group): recipient-field text. */
  onWriteMany: (to: string) => void;
}

/**
 * The right-hand pane. A contact being viewed is remembered as it was when
 * opened (`shown`): after a save, or when the search no longer matches it, it
 * is not in the list to look up, and the pane must not go blank (on a phone
 * that is a screen with no way back).
 */
type Pane =
  | { kind: 'none' }
  | { kind: 'view'; id: string; shown: Contact }
  | { kind: 'edit'; input: ContactInput; from: Contact | null }
  | { kind: 'group'; id: string };

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
export function ContactsView({ accounts, phone, onClose, onWrite, onFindMail, onWriteMany }: Props) {
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
  const [groups, setGroups] = useState<ContactGroup[]>([]);
  const [everyone, setEveryone] = useState<Contact[]>([]);
  const [categories, setCategories] = useState<Category[]>([]);
  const [naming, setNaming] = useState(false);
  const files = useFileTasks();
  /** Import or export is asking which address book. */
  const [transfer, setTransfer] = useState<'import' | 'export' | null>(null);

  useEffect(() => {
    let live = true;
    api.listCategories().then(
      (list) => {
        if (live) setCategories(list);
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api]);

  // Groups, and everyone who could be in one (whatever the search).
  useEffect(() => {
    let live = true;
    Promise.all([api.listContactGroups(), api.listContacts('')]).then(
      ([nextGroups, all]) => {
        if (!live) return;
        setGroups(nextGroups);
        setEveryone(all);
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api, revision]);

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
      from: null,
    });
  };

  // The list's copy is the fresh one; fall back to the copy the pane opened with.
  const selected = pane.kind === 'view' ? ((contacts ?? []).find((c) => c.id === pane.id) ?? pane.shown) : undefined;
  const showList = !phone || pane.kind === 'none';
  const showPane = !phone || pane.kind !== 'none';
  const favourites = (contacts ?? []).filter((c) => c.favorite);
  const others = (contacts ?? []).filter((c) => !c.favorite);
  const openGroup = pane.kind === 'group' ? groups.find((g) => g.id === pane.id) : undefined;

  const setFavourite = (c: Contact, favorite: boolean) => {
    setError(null);
    api.saveContact({ ...inputOf(c), favorite }).then(
      (next) => {
        setRevision((r) => r + 1);
        if (pane.kind === 'view' && pane.id === c.id) setPane({ kind: 'view', id: c.id, shown: next });
      },
      (e: unknown) => {
        setError(toAppError(e));
      },
    );
  };

  const row = (c: Contact) => (
    <li key={c.id}>
      <button
        type="button"
        className={`contact-row${pane.kind === 'view' && pane.id === c.id ? ' contact-row--active' : ''}`}
        aria-current={pane.kind === 'view' && pane.id === c.id ? 'true' : undefined}
        onClick={() => {
          setError(null);
          setPane({ kind: 'view', id: c.id, shown: c });
        }}
      >
        <Avatar name={c.name} address={c.emails[0] ?? c.name} photo={c.photo} />
        <span className="contact-row__text">
          <span className="contact-row__name">{displayName(c)}</span>
          <span className="muted small">{c.emails[0] ?? c.phones[0] ?? c.organization}</span>
        </span>
        {c.favorite && (
          <span className="contact-row__star" title="Favourite">
            <Icon name="star" size={14} filled />
          </span>
        )}
        {c.state !== 'synced' && (
          <span className="contact-row__pending" title="Waiting to sync">
            <Icon name="clock" size={14} />
          </span>
        )}
      </button>
    </li>
  );

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
        <MenuButton label="More contact options" icon="more" title="Import and export">
          <MenuItem
            icon="upload"
            onSelect={() => {
              setTransfer('import');
            }}
          >
            Import contacts (.vcf)…
          </MenuItem>
          <MenuItem
            icon="download"
            onSelect={() => {
              setTransfer('export');
            }}
          >
            Export contacts…
          </MenuItem>
        </MenuButton>
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
            <ConnectCards
              accounts={accounts}
              statuses={books}
              onConnected={() => {
                setRevision((r) => r + 1);
              }}
            />
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
                {favourites.length > 0 && (
                  <>
                    <h2 className="contacts__subhead">Favourites</h2>
                    <ul className="contacts__list" aria-label="Favourites">
                      {favourites.map(row)}
                    </ul>
                  </>
                )}
                {others.length > 0 && (
                  <>
                    {favourites.length > 0 && <h2 className="contacts__subhead">Everyone else</h2>}
                    <ul className="contacts__list" aria-label="Contacts">
                      {others.map(row)}
                    </ul>
                  </>
                )}
                {!search.trim() && (
                  <div className="contacts__groups">
                    <h2 className="contacts__subhead contacts__subhead--row">
                      Groups
                      <button
                        type="button"
                        className="btn btn--link btn--small"
                        onClick={() => {
                          setNaming(true);
                        }}
                      >
                        <Icon name="plus" size={14} />
                        New group
                      </button>
                    </h2>
                    {groups.length === 0 ? (
                      <p className="muted small contacts__hint">Put people in a group to write to them all at once.</p>
                    ) : (
                      <ul className="contacts__list" aria-label="Groups">
                        {groups.map((g) => (
                          <li key={g.id}>
                            <button
                              type="button"
                              className={`contact-row${openGroup?.id === g.id ? ' contact-row--active' : ''}`}
                              aria-current={openGroup?.id === g.id ? 'true' : undefined}
                              onClick={() => {
                                setError(null);
                                setPane({ kind: 'group', id: g.id });
                              }}
                            >
                              <span className="avatar avatar--group" aria-hidden="true">
                                <Icon name="people" size={18} />
                              </span>
                              <span className="contact-row__text">
                                <span className="contact-row__name">{g.name}</span>
                                <span className="muted small">
                                  {g.memberIds.length === 1 ? '1 person' : `${String(g.memberIds.length)} people`}
                                </span>
                              </span>
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
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
                    from: selected,
                  });
                }}
                onDeleted={() => {
                  setPane({ kind: 'none' });
                  setRevision((r) => r + 1);
                }}
                onFavourite={(on) => {
                  setFavourite(selected, on);
                }}
                categories={categories}
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
                categories={categories}
                onCancel={() => {
                  setPane(pane.from ? { kind: 'view', id: pane.from.id, shown: pane.from } : { kind: 'none' });
                }}
                onSaved={(contact) => {
                  setRevision((r) => r + 1);
                  setPane({ kind: 'view', id: contact.id, shown: contact });
                }}
              />
            )}
            {pane.kind === 'group' && openGroup && (
              <GroupPane
                key={openGroup.id}
                group={openGroup}
                everyone={everyone}
                phone={phone}
                onBack={() => {
                  setPane({ kind: 'none' });
                }}
                onChanged={setGroups}
                onDeleted={(next) => {
                  setGroups(next);
                  setPane({ kind: 'none' });
                }}
                onWriteMany={onWriteMany}
              />
            )}
          </div>
        )}
      </div>
      {naming && (
        <GroupNameDialog
          title="New group"
          initial=""
          onClose={() => {
            setNaming(false);
          }}
          onSave={(name) =>
            api.saveContactGroup({ id: null, accountId: null, name, memberIds: [] }).then((next) => {
              setNaming(false);
              setGroups(next);
              const made = next.find((g) => g.name.toLocaleLowerCase() === name.trim().toLocaleLowerCase());
              if (made) setPane({ kind: 'group', id: made.id });
            })
          }
        />
      )}

      {transfer && (
        <AddressBookPicker
          mode={transfer}
          title={transfer === 'import' ? 'Import contacts (.vcf)' : 'Export contacts'}
          accounts={accounts}
          initial={syncedBooks[0]?.id ?? null}
          onClose={() => {
            setTransfer(null);
          }}
          onPick={(accountId) => {
            const mode = transfer;
            setTransfer(null);
            files.run(
              mode === 'import'
                ? importContactsTask(api, accounts, accountId, () => {
                    setRevision((r) => r + 1);
                  })
                : exportContactsTask(api, accounts, accountId),
            );
          }}
        />
      )}
    </section>
  );
}

/** What the editor needs to save a contact as it is. */
function inputOf(c: Contact): ContactInput {
  return {
    id: c.id,
    accountId: c.accountId,
    name: c.name,
    emails: c.emails,
    phones: c.phones,
    organization: c.organization,
    title: c.title,
    notes: c.notes,
    favorite: c.favorite,
    categories: c.categories,
  };
}

/** Recipient text for everyone in a group with an address. */
export function groupRecipients(group: ContactGroup, everyone: readonly Contact[]): string {
  return group.memberIds
    .map((id) => everyone.find((c) => c.id === id))
    .filter((c): c is Contact => c !== undefined && c.emails.length > 0)
    .map((c) => {
      const name = c.name.replace(/["<>]/g, '').trim();
      const address = c.emails[0] ?? '';
      if (!name) return address;
      return /[,;]/.test(name) ? `"${name}" <${address}>` : `${name} <${address}>`;
    })
    .join(', ');
}

function GroupNameDialog({
  title,
  initial,
  onClose,
  onSave,
}: {
  title: string;
  initial: string;
  onClose: () => void;
  onSave: (name: string) => Promise<unknown>;
}) {
  const [name, setName] = useState(initial);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    onSave(name).then(
      () => {
        setBusy(false);
      },
      (err: unknown) => {
        setBusy(false);
        setError(toAppError(err));
      },
    );
  };
  return (
    <Modal title={title} onClose={onClose} size="small">
      <form onSubmit={submit} noValidate aria-label={title}>
        <label className="form-field">
          <span className="form-field__label">Group name</span>
          <input
            className="field"
            value={name}
            maxLength={200}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </label>
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
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** A contact group: its people, adding and removing them, renaming, deleting, and writing to them all. */
function GroupPane({
  group,
  everyone,
  phone,
  onBack,
  onChanged,
  onDeleted,
  onWriteMany,
}: {
  group: ContactGroup;
  everyone: readonly Contact[];
  phone: boolean;
  onBack: () => void;
  onChanged: (groups: ContactGroup[]) => void;
  onDeleted: (groups: ContactGroup[]) => void;
  onWriteMany: (to: string) => void;
}) {
  const api = useMailApi();
  const id = useId();
  const [adding, setAdding] = useState('');
  const [renaming, setRenaming] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const members = group.memberIds
    .map((m) => everyone.find((c) => c.id === m))
    .filter((c): c is Contact => c !== undefined);
  const candidates = everyone.filter((c) => !group.memberIds.includes(c.id));
  const to = groupRecipients(group, everyone);

  const saveMembers = (memberIds: string[]) => {
    setError(null);
    api.saveContactGroup({ ...group, memberIds }).then(onChanged, (e: unknown) => {
      setError(toAppError(e));
    });
  };

  return (
    <article className="contact-card group-card" aria-label={group.name}>
      {phone && (
        <button type="button" className="icon-btn contact-card__back" aria-label="Back to contacts" onClick={onBack}>
          <Icon name="back" />
        </button>
      )}
      <div className="contact-card__head">
        <span className="avatar avatar--group avatar--big" aria-hidden="true">
          <Icon name="people" size={30} />
        </span>
        <div>
          <h2 className="contact-card__name">{group.name}</h2>
          <p className="muted small">
            {members.length === 1 ? '1 person' : `${String(members.length)} people`}
            {group.accountId === null ? ' · on this device' : ''}
          </p>
        </div>
      </div>
      <div className="contact-card__actions">
        <button
          type="button"
          className="btn btn--primary"
          disabled={!to}
          onClick={() => {
            onWriteMany(to);
          }}
        >
          <Icon name="pen" />
          Email group
        </button>
        <button
          type="button"
          className="btn btn--secondary"
          onClick={() => {
            setRenaming(true);
          }}
        >
          Rename
        </button>
        <button
          type="button"
          className="btn btn--link btn--danger-text"
          onClick={() => {
            setConfirm(true);
          }}
        >
          Delete group
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      <h3 className="contacts__subhead">People in this group</h3>
      {members.length === 0 ? (
        <p className="muted">Nobody yet. Add people below.</p>
      ) : (
        <ul className="contacts__list" aria-label={`People in ${group.name}`}>
          {members.map((c) => (
            <li key={c.id} className="contact-row contact-row--static">
              <Avatar name={c.name} address={c.emails[0] ?? c.name} photo={c.photo} />
              <span className="contact-row__text">
                <span className="contact-row__name">{displayName(c)}</span>
                <span className="muted small">{c.emails[0] ?? ''}</span>
              </span>
              <button
                type="button"
                className="btn btn--link btn--small"
                aria-label={`Remove ${displayName(c)} from ${group.name}`}
                onClick={() => {
                  saveMembers(group.memberIds.filter((m) => m !== c.id));
                }}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {candidates.length > 0 && (
        <form
          className="group-card__add"
          aria-label="Add someone"
          onSubmit={(e) => {
            e.preventDefault();
            if (adding) saveMembers([...group.memberIds, adding]);
            setAdding('');
          }}
        >
          <label className="form-field">
            <span className="form-field__label" id={`${id}-add`}>
              Add someone
            </span>
            <select
              className="field"
              aria-labelledby={`${id}-add`}
              value={adding}
              onChange={(e) => {
                setAdding(e.target.value);
              }}
            >
              <option value="">Choose a contact…</option>
              {candidates.map((c) => (
                <option key={c.id} value={c.id}>
                  {displayName(c)}
                  {c.emails[0] && c.name ? ` (${c.emails[0]})` : ''}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className="btn btn--secondary" disabled={!adding}>
            Add
          </button>
        </form>
      )}
      {renaming && (
        <GroupNameDialog
          title="Rename group"
          initial={group.name}
          onClose={() => {
            setRenaming(false);
          }}
          onSave={(name) =>
            api.saveContactGroup({ ...group, name }).then((next) => {
              setRenaming(false);
              onChanged(next);
            })
          }
        />
      )}
      {confirm && (
        <ConfirmDialog
          title={`Delete ${group.name}?`}
          description="The people in it stay in your contacts."
          confirmLabel="Delete group"
          danger
          onCancel={() => {
            setConfirm(false);
          }}
          onConfirm={() => {
            setConfirm(false);
            api.deleteContactGroup(group.id).then(onDeleted, (e: unknown) => {
              setError(toAppError(e));
            });
          }}
        />
      )}
    </article>
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
  onFavourite,
  categories,
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
  onFavourite: (on: boolean) => void;
  categories: readonly Category[];
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
        <Avatar name={contact.name} address={contact.emails[0] ?? display} size={64} photo={contact.photo} />
        <div className="contact-card__who">
          <h2 className="contact-card__name">
            {display}
            <button
              type="button"
              className={`icon-btn contact-card__star${contact.favorite ? ' contact-card__star--on' : ''}`}
              aria-pressed={contact.favorite}
              aria-label={contact.favorite ? 'Remove from favourites' : 'Add to favourites'}
              title={contact.favorite ? 'Remove from favourites' : 'Add to favourites'}
              onClick={() => {
                onFavourite(!contact.favorite);
              }}
            >
              <Icon name="star" filled={contact.favorite} />
            </button>
          </h2>
          {role && <p className="muted">{role}</p>}
          <p className="muted small">
            {where}
            {contact.state !== 'synced' && ' · waiting to sync'}
          </p>
          {contact.categories.length > 0 && (
            <ul className="contact-card__categories" aria-label="Categories">
              {contact.categories.map((c) => (
                <li
                  key={c}
                  className="category-chip"
                  style={{ ['--category' as string]: categories.find((x) => x.name === c)?.color ?? 'var(--muted)' }}
                >
                  {c}
                </li>
              ))}
            </ul>
          )}
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
  categories,
  onCancel,
  onSaved,
}: {
  initial: ContactInput;
  /** Accounts whose address book syncs; a new contact may go in one of them. */
  choices: readonly Account[];
  categories: readonly Category[];
  onCancel: () => void;
  onSaved: (contact: Contact) => void;
}) {
  const api = useMailApi();
  const id = useId();
  const [input, setInput] = useState<ContactInput>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const isNew = initial.id === undefined;
  const fileRef = useRef<HTMLInputElement>(null);
  const [photo, setPhoto] = useState<string | null>(initial.photo ?? null);
  const names = [...new Set([...categories.map((c) => c.name), ...(input.categories ?? [])])];

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
    api.saveContact({ ...input, photo }).then(onSaved, (e: unknown) => {
      setError(toAppError(e));
      setBusy(false);
    });
  };

  return (
    <form className="contact-editor" aria-labelledby={`${id}-title`} noValidate onSubmit={submit}>
      <h2 id={`${id}-title`} className="contact-card__name">
        {isNew ? 'New contact' : 'Edit contact'}
      </h2>
      <div className="contact-editor__photo">
        <Avatar name={input.name} address={input.emails[0] ?? input.name} size={56} photo={photo} />
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          className="visually-hidden"
          aria-label="Choose a photo"
          tabIndex={-1}
          onChange={(e) => {
            const file = e.target.files?.[0];
            e.target.value = '';
            if (!file) return;
            setError(null);
            photoFromFile(file).then(setPhoto, (err: unknown) => {
              setError({ kind: 'invalid', message: err instanceof Error ? err.message : 'That picture could not be used.' });
            });
          }}
        />
        <button
          type="button"
          className="btn btn--secondary btn--small"
          onClick={() => {
            fileRef.current?.click();
          }}
        >
          {photo ? 'Change photo' : 'Add photo'}
        </button>
        {photo && (
          <button
            type="button"
            className="btn btn--link btn--small"
            onClick={() => {
              setPhoto(null);
            }}
          >
            Remove photo
          </button>
        )}
      </div>
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
      <label className="choice">
        <input
          type="checkbox"
          checked={input.favorite ?? false}
          onChange={(e) => {
            set({ favorite: e.target.checked });
          }}
        />
        Favourite
      </label>
      {names.length > 0 && (
        <fieldset className="contact-editor__categories">
          <legend className="form-field__label">Categories</legend>
          {names.map((name) => (
            <label key={name} className="choice">
              <input
                type="checkbox"
                checked={(input.categories ?? []).includes(name)}
                onChange={(e) => {
                  const chosen = input.categories ?? [];
                  set({ categories: e.target.checked ? [...chosen, name] : chosen.filter((c) => c !== name) });
                }}
              />
              {name}
            </label>
          ))}
        </fieldset>
      )}
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
