import { useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import type { Mailbox } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { addressOf, isPlausibleAddress, splitRecipients } from './recipients';

interface Props {
  id: string;
  /** The field's name, for the chip list ("To recipients"). */
  label: string;
  /** The whole field as text, "A <a@x>, b@y". The draft keeps this form. */
  value: string;
  onChange: (value: string) => void;
  invalid: boolean;
  describedBy: string | undefined;
  autoFocus: boolean;
}

const join = (entries: readonly string[]) => entries.join(', ');

/** Whether quotes and angle brackets are closed, so a comma ends an entry. */
function balanced(text: string): boolean {
  let quoted = false;
  let angled = false;
  for (const ch of text) {
    if (ch === '"' && !angled) quoted = !quoted;
    else if (ch === '<' && !quoted) angled = true;
    else if (ch === '>' && !quoted) angled = false;
  }
  return !quoted && !angled;
}

/** "Name <a@x>" shows as Name; a bare address shows as itself. */
function chipLabel(entry: string): string {
  const angle = /^(.*?)<[^<>]*>\s*$/.exec(entry);
  const name = angle?.[1]?.trim().replace(/^"|"$/g, '');
  return name !== undefined && name !== '' ? name : entry;
}

/** A suggestion as a recipient entry; names with a comma are quoted so they stay one entry. */
function entryFor(person: Mailbox): string {
  const name = person.name?.replace(/["<>]/g, '').trim();
  if (!name) return person.address;
  return /[,;]/.test(name) ? `"${name}" <${person.address}>` : `${name} <${person.address}>`;
}

/** Wait this long after typing stops before looking people up. */
const SUGGEST_DELAY_MS = 120;

/**
 * Recipients as removable chips, with a text box for the one being typed.
 * A comma, semicolon, Enter or leaving the field turns the text into a chip;
 * Backspace on an empty box takes the last chip back for editing. While
 * typing, people you have written to or heard from are suggested below.
 */
export function RecipientField({ id, label, value, onChange, invalid, describedBy, autoFocus }: Props) {
  const api = useMailApi();
  const [text, setText] = useState('');
  const all = splitRecipients(value);
  const chips = text.trim() === '' ? all : all.slice(0, -1);
  const [suggestions, setSuggestions] = useState<Mailbox[]>([]);
  const [active, setActive] = useState(-1);
  const [open, setOpen] = useState(false);
  const request = useRef(0);
  const listId = `${id}-suggestions`;

  // Look people up as the user types (newest answer wins).
  const query = text.trim();
  useEffect(() => {
    const ticket = ++request.current;
    if (query === '') return;
    const timer = window.setTimeout(() => {
      api.suggestRecipients(query).then(
        (people) => {
          if (ticket !== request.current) return;
          const taken = new Set(chips.map((c) => addressOf(c)?.toLowerCase()));
          setSuggestions(people.filter((p) => !taken.has(p.address.toLowerCase())));
          setActive(-1);
        },
        () => undefined,
      );
    }, SUGGEST_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
    };
    // `chips` is derived from `value`; re-run only when the typed text changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, query]);
  const showList = open && query !== '' && suggestions.length > 0;

  const pick = (person: Mailbox) => {
    emit([...chips, entryFor(person)], '');
    setSuggestions([]);
    setActive(-1);
  };

  const emit = (entries: readonly string[], typing: string) => {
    setText(typing);
    onChange(join(typing.trim() === '' ? entries : [...entries, typing.trim()]));
  };

  const type = (next: string) => {
    if (!balanced(next) || !/[,;\n]/.test(next)) {
      emit(chips, next);
      return;
    }
    const parts = splitRecipients(next);
    const ended = /[,;\n]\s*$/.test(next);
    const rest = ended ? '' : (parts.pop() ?? '');
    emit([...chips, ...parts], rest);
  };

  const commit = () => {
    if (text.trim() !== '') emit([...chips, text.trim()], '');
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (showList && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setActive((i) => (i + step + suggestions.length) % suggestions.length);
      return;
    }
    const chosen = showList ? suggestions[active] : undefined;
    if (chosen && (event.key === 'Enter' || event.key === 'Tab')) {
      event.preventDefault();
      pick(chosen);
      return;
    }
    if (showList && event.key === 'Escape') {
      // Close the list only; a second Escape closes compose as before.
      event.stopPropagation();
      setOpen(false);
      return;
    }
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && text.trim() !== '') {
      event.preventDefault();
      commit();
    } else if (event.key === 'Backspace' && text === '' && chips.length > 0) {
      event.preventDefault();
      const last = chips.at(-1) ?? '';
      emit(chips.slice(0, -1), last);
    }
  };

  const onPaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text');
    if (!/[,;\n]/.test(pasted)) return;
    event.preventDefault();
    type(`${text}${pasted},`);
  };

  return (
    <div className="recipients">
      {chips.length > 0 && (
        <ul className="recipients__list" aria-label={`${label} recipients`}>
          {chips.map((entry, index) => {
            const address = addressOf(entry);
            const ok = address !== null && isPlausibleAddress(address);
            return (
              // Entries are de-duplicated on send, not here, so the index keeps keys unique.
              <li key={`${index}-${entry}`} className={ok ? 'chip' : 'chip chip--bad'} title={entry}>
                <span className="chip__text">{chipLabel(entry)}</span>
                <button
                  type="button"
                  className="chip__remove"
                  aria-label={`Remove ${entry}`}
                  onClick={() => {
                    emit(
                      chips.filter((_, i) => i !== index),
                      text,
                    );
                  }}
                >
                  <Icon name="close" size={12} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <input
        id={id}
        role="combobox"
        aria-expanded={showList}
        aria-controls={showList ? listId : undefined}
        aria-autocomplete="list"
        aria-activedescendant={showList && active >= 0 ? `${listId}-${String(active)}` : undefined}
        type="text"
        inputMode="email"
        autoComplete="email"
        className="compose__input recipients__input"
        value={text}
        aria-invalid={invalid ? true : undefined}
        aria-describedby={describedBy}
        // The first field of a view the user just opened.
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={autoFocus}
        onChange={(e) => {
          type(e.target.value);
          setOpen(true);
        }}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onBlur={() => {
          setOpen(false);
          commit();
        }}
      />
      {showList && (
        <div id={listId} role="listbox" className="suggestions" aria-label="Suggested recipients">
          {suggestions.map((person, index) => (
            // Options are picked with the pointer or the keyboard (handled on the input).
            // eslint-disable-next-line jsx-a11y/click-events-have-key-events
            <div
              key={person.address}
              id={`${listId}-${String(index)}`}
              role="option"
              // Focus stays in the field (aria-activedescendant); options are not tab stops.
              tabIndex={-1}
              aria-selected={index === active}
              className="suggestion"
              // Keep focus in the field, so picking doesn't first commit the half-typed text.
              onMouseDown={(e) => {
                e.preventDefault();
              }}
              onClick={() => {
                pick(person);
              }}
            >
              <span className="suggestion__name">{person.name ?? person.address}</span>
              {person.name && person.name.toLowerCase() !== person.address.toLowerCase() && (
                <span className="suggestion__address">{person.address}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
