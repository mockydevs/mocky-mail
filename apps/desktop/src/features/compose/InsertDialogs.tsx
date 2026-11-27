import { useEffect, useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Contact, DraftAttachment, Mailbox, MessageListItem } from '../../api/types';
import { formatLongDate } from '../../lib/format';
import { Modal } from '../../ui/Modal';

/** Insert › Symbol: the ones people reach for, with names to search by. */
export const SYMBOLS: readonly [string, string][] = [
  ['©', 'Copyright'],
  ['®', 'Registered'],
  ['™', 'Trade mark'],
  ['€', 'Euro'],
  ['£', 'Pound'],
  ['¥', 'Yen'],
  ['°', 'Degree'],
  ['±', 'Plus or minus'],
  ['×', 'Multiply times'],
  ['÷', 'Divide'],
  ['→', 'Right arrow'],
  ['←', 'Left arrow'],
  ['↑', 'Up arrow'],
  ['↓', 'Down arrow'],
  ['✓', 'Check mark tick'],
  ['✗', 'Cross mark'],
  ['•', 'Bullet'],
  ['…', 'Ellipsis dots'],
  ['–', 'En dash'],
  ['—', 'Em dash'],
  ['«', 'Left guillemet quote'],
  ['»', 'Right guillemet quote'],
  ['“', 'Left double quote'],
  ['”', 'Right double quote'],
  ['§', 'Section'],
  ['¶', 'Paragraph pilcrow'],
  ['½', 'One half'],
  ['¼', 'One quarter'],
  ['¾', 'Three quarters'],
  ['≈', 'Approximately'],
  ['≠', 'Not equal'],
  ['≤', 'Less than or equal'],
  ['≥', 'Greater than or equal'],
  ['µ', 'Micro'],
  ['¢', 'Cent'],
  ['$', 'Dollar'],
];

export function SymbolDialog({ onPick, onClose }: { onPick: (symbol: string) => void; onClose: () => void }) {
  const id = useId();
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  const shown = SYMBOLS.filter(([symbol, name]) => q === '' || symbol === q || name.toLowerCase().includes(q));
  return (
    <Modal title="Insert a symbol" size="small" onClose={onClose}>
      <label htmlFor={`${id}-search`} className="visually-hidden">
        Search symbols
      </label>
      <input
        id={`${id}-search`}
        className="field"
        type="search"
        placeholder="Search symbols"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
        }}
      />
      {shown.length === 0 ? (
        <p className="form-hint">No symbol by that name.</p>
      ) : (
        <div className="symbol-grid" role="group" aria-label="Symbols">
          {shown.map(([symbol, name]) => (
            <button
              key={symbol}
              type="button"
              className="symbol-grid__btn"
              aria-label={`${name} ${symbol}`}
              title={name}
              onClick={() => {
                onPick(symbol);
              }}
            >
              {symbol}
            </button>
          ))}
        </div>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Close
        </button>
      </div>
    </Modal>
  );
}

/** A list to pick one thing from, loaded for a search; picking makes the attachment. */
function AttachPicker<T>({
  title,
  searchLabel,
  empty,
  load,
  itemKey,
  itemLabel,
  itemDetail,
  attach,
  onAttach,
  onClose,
}: {
  title: string;
  searchLabel: string;
  empty: string;
  load: (query: string) => Promise<T[]>;
  itemKey: (item: T) => string;
  itemLabel: (item: T) => string;
  itemDetail: (item: T) => string;
  attach: (item: T) => Promise<DraftAttachment>;
  onAttach: (file: DraftAttachment) => void;
  onClose: () => void;
}) {
  const id = useId();
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<T[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    load(query).then(
      (found) => {
        if (active) setItems(found);
      },
      (e: unknown) => {
        if (active) setError(toAppError(e));
      },
    );
    return () => {
      active = false;
    };
    // `load` is a new function each render; the search text is what matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  const pick = (item: T) => {
    setBusy(true);
    setError(null);
    attach(item).then(
      (file) => {
        onAttach(file);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  return (
    <Modal title={title} size="medium" onClose={onClose}>
      <label htmlFor={`${id}-search`} className="visually-hidden">
        {searchLabel}
      </label>
      <input
        id={`${id}-search`}
        className="field"
        type="search"
        placeholder={searchLabel}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
        }}
      />
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {items === null ? (
        <p className="form-hint" aria-busy="true">
          Loading
        </p>
      ) : items.length === 0 ? (
        <p className="form-hint">{empty}</p>
      ) : (
        <ul className="pick-list" aria-label={title}>
          {items.map((item) => (
            <li key={itemKey(item)}>
              <button
                type="button"
                className="pick-list__item"
                disabled={busy}
                onClick={() => {
                  pick(item);
                }}
              >
                <span className="pick-list__name">{itemLabel(item)}</span>
                <span className="pick-list__detail muted small">{itemDetail(item)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
      </div>
    </Modal>
  );
}

export function ContactCardDialog({ onAttach, onClose }: { onAttach: (file: DraftAttachment) => void; onClose: () => void }) {
  const api = useMailApi();
  return (
    <AttachPicker<Contact>
      title="Attach a contact card"
      searchLabel="Search contacts"
      empty="No contacts found."
      load={(query) => api.listContacts(query)}
      itemKey={(c) => c.id}
      itemLabel={(c) => c.name || (c.emails[0] ?? 'No name')}
      itemDetail={(c) => [c.emails[0], c.organization].filter(Boolean).join(' · ')}
      attach={(c) => api.contactCardAttachment(c.id)}
      onAttach={onAttach}
      onClose={onClose}
    />
  );
}

function senderOf(m: MessageListItem): string {
  const from: Mailbox | null = m.from;
  return from ? (from.name ?? from.address) : '';
}

export function ItemDialog({ onAttach, onClose }: { onAttach: (file: DraftAttachment) => void; onClose: () => void }) {
  const api = useMailApi();
  return (
    <AttachPicker<MessageListItem>
      title="Attach a message"
      searchLabel="Search messages"
      empty="No messages found."
      load={(query) =>
        api
          .listMessages({
            accountId: null,
            folderId: null,
            role: null,
            filter: 'all',
            search: query.trim() === '' ? null : query.trim(),
            searchScope: 'all',
            offset: 0,
            limit: 30,
          })
          .then((page) => page.items)
      }
      itemKey={(m) => m.id}
      itemLabel={(m) => m.subject || '(no subject)'}
      itemDetail={(m) => [senderOf(m), formatLongDate(m.date)].filter(Boolean).join(' · ')}
      attach={(m) => api.messageAsAttachment(m.id)}
      onAttach={onAttach}
      onClose={onClose}
    />
  );
}

/** Check Names: which of several contacts a typed name means. */
export function NameChooser({
  input,
  candidates,
  onPick,
  onCancel,
}: {
  input: string;
  candidates: readonly Mailbox[];
  onPick: (person: Mailbox) => void;
  onCancel: () => void;
}) {
  return (
    <Modal title={`Who is “${input}”?`} description="More than one of your contacts matches. Pick one." size="small" onClose={onCancel}>
      <ul className="pick-list" aria-label={`Matches for ${input}`}>
        {candidates.map((person) => (
          <li key={person.address}>
            <button
              type="button"
              className="pick-list__item"
              onClick={() => {
                onPick(person);
              }}
            >
              <span className="pick-list__name">{person.name ?? person.address}</span>
              <span className="pick-list__detail muted small">{person.address}</span>
            </button>
          </li>
        ))}
      </ul>
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onCancel}>
          Leave it for now
        </button>
      </div>
    </Modal>
  );
}
