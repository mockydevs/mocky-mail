import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type Ref } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import type { Category, Folder, FolderRole } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { ROLE_TITLES } from '../mailbox/selectors';
import { appendTerm, applySuggestion, HAS_VALUES, IS_VALUES, operatorAt, SEARCH_HELP, type OperatorContext } from './query';

/** A value offered for the operator being typed. */
interface Suggestion {
  value: string;
  label: string;
  detail?: string;
}

const SUGGEST_DELAY_MS = 150;
const IN_ROLES: readonly FolderRole[] = ['inbox', 'drafts', 'sent', 'archive', 'junk', 'trash'];
const IS_DETAILS: Readonly<Record<string, string>> = {
  unread: 'Not read yet',
  read: 'Already read',
  flagged: 'Flagged for follow-up',
  pinned: 'Pinned to the top',
  answered: 'You replied',
};

interface Props {
  value: string;
  onChange: (value: string) => void;
  /** Lets the "/" shortcut focus the box. */
  inputRef: Ref<HTMLInputElement>;
  placeholder: string;
  /** For `category:` suggestions. */
  categories: readonly Category[];
  /** For `in:` suggestions: folders the user made. */
  folders: readonly Folder[];
}

function setRef<T>(ref: Ref<T> | undefined, value: T | null): void {
  if (typeof ref === 'function') ref(value);
  else if (ref) ref.current = value;
}

const startsOrHas = (text: string, partial: string) => {
  const t = text.toLowerCase();
  const p = partial.trim().toLowerCase();
  return p === '' || t.startsWith(p) || t.includes(` ${p}`);
};

/** What to offer for the value of `context`'s operator (people come in separately). */
function localSuggestions(context: OperatorContext, categories: readonly Category[], folders: readonly Folder[]): Suggestion[] {
  const { field, partial } = context;
  switch (field) {
    case 'is':
      return IS_VALUES.filter((v) => startsOrHas(v, partial)).map((v) => ({ value: v, label: v, detail: IS_DETAILS[v] ?? '' }));
    case 'has':
      return HAS_VALUES.filter((v) => startsOrHas(v, partial)).map((v) => ({ value: v, label: v, detail: 'Has files attached' }));
    case 'category':
      return categories.filter((c) => startsOrHas(c.name, partial)).map((c) => ({ value: c.name, label: c.name }));
    case 'in': {
      const roles = IN_ROLES.filter((r) => startsOrHas(r, partial) || startsOrHas(ROLE_TITLES[r], partial)).map((r) => ({
        value: r,
        label: r,
        detail: ROLE_TITLES[r],
      }));
      const names = [...new Set(folders.filter((f) => f.role === 'other').map((f) => f.name))]
        .filter((n) => startsOrHas(n, partial))
        .map((n) => ({ value: n, label: n, detail: 'Folder' }));
      return [...roles, ...names].slice(0, 10);
    }
    case 'before':
    case 'after':
      return ['today', 'yesterday'].filter((v) => startsOrHas(v, partial)).map((v) => ({ value: v, label: v }));
    case 'from':
    case 'to':
    case 'cc':
    case 'subject':
    case 'body':
    case 'larger':
    case 'smaller':
      return [];
  }
}

/**
 * The search box: the query language, with suggestions for the operator
 * being typed (people, categories, folders, values) and a "Search tips"
 * popover whose examples go into the query. Esc closes what is open, then
 * clears the search.
 */
export function SearchField({ value, onChange, inputRef, placeholder, categories, folders }: Props) {
  const api = useMailApi();
  const id = useId();
  const listId = `${id}-suggestions`;
  const helpId = `${id}-help`;
  const own = useRef<HTMLInputElement | null>(null);
  const [caret, setCaret] = useState(0);
  const [focused, setFocused] = useState(false);
  /** Esc closed the suggestions; typing opens them again. */
  const [dismissed, setDismissed] = useState(false);
  const [active, setActive] = useState(-1);
  const [help, setHelp] = useState(false);
  const [people, setPeople] = useState<{ key: string; list: Suggestion[] }>({ key: '', list: [] });
  const pendingCaret = useRef<number | null>(null);
  const request = useRef(0);
  const helpRef = useRef<HTMLDivElement>(null);
  const helpButton = useRef<HTMLButtonElement>(null);

  const context = operatorAt(value, Math.min(caret, value.length));
  const personField = context && (context.field === 'from' || context.field === 'to' || context.field === 'cc');
  const peopleKey = personField ? context.partial.trim() : '';

  // People for from:, to: and cc: (newest answer wins).
  useEffect(() => {
    const ticket = ++request.current;
    if (peopleKey === '') return;
    const timer = window.setTimeout(() => {
      api.suggestRecipients(peopleKey).then(
        (found) => {
          if (ticket !== request.current) return;
          setPeople({
            key: peopleKey,
            list: found.map((p) => ({ value: p.address, label: p.name ?? p.address, ...(p.name ? { detail: p.address } : {}) })),
          });
        },
        () => undefined,
      );
    }, SUGGEST_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [api, peopleKey]);

  const options: Suggestion[] = !context
    ? []
    : personField
      ? people.key === peopleKey && peopleKey !== ''
        ? people.list
        : []
      : localSuggestions(context, categories, folders);
  // Nothing to offer once the value is typed in full.
  const useful = options.filter((o) => o.value.toLowerCase() !== context?.partial.trim().toLowerCase() || options.length > 1);
  const open = focused && !dismissed && useful.length > 0;
  const activeIndex = open && active < useful.length ? active : -1;

  // Put the caret back after a suggestion or example changed the text.
  useLayoutEffect(() => {
    const at = pendingCaret.current;
    const input = own.current;
    if (at === null || !input) return;
    pendingCaret.current = null;
    input.setSelectionRange(at, at);
  });

  // The tips close on a click elsewhere.
  useEffect(() => {
    if (!help) return;
    const outside = (event: Event) => {
      if (event.target instanceof Node && (helpRef.current?.contains(event.target) || helpButton.current?.contains(event.target))) return;
      setHelp(false);
    };
    // Esc inside the tips closes them only, not the screen underneath.
    const popover = helpRef.current;
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.stopPropagation();
      setHelp(false);
      helpButton.current?.focus();
    };
    document.addEventListener('mousedown', outside);
    document.addEventListener('touchstart', outside);
    popover?.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('mousedown', outside);
      document.removeEventListener('touchstart', outside);
      popover?.removeEventListener('keydown', escape);
    };
  }, [help]);

  const change = (next: string, at: number) => {
    onChange(next);
    setCaret(at);
    setDismissed(false);
    setActive(-1);
  };

  const pick = (option: Suggestion) => {
    if (!context) return;
    const next = applySuggestion(value, context, option.value);
    pendingCaret.current = next.caret;
    change(next.query, next.caret);
  };

  const insertExample = (example: string) => {
    const next = appendTerm(value, example);
    pendingCaret.current = next.length;
    change(next, next.length);
    setHelp(false);
    own.current?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setActive((i) => (Math.max(i, step > 0 ? -1 : 0) + step + useful.length) % useful.length);
      return;
    }
    const chosen = activeIndex >= 0 ? useful[activeIndex] : undefined;
    if (chosen && (event.key === 'Enter' || event.key === 'Tab')) {
      event.preventDefault();
      pick(chosen);
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      if (open) setDismissed(true);
      else if (help) setHelp(false);
      else if (value !== '') change('', 0);
    }
  };

  const track = () => {
    setCaret(own.current?.selectionStart ?? value.length);
  };

  return (
    <div className="search" role="search">
      <Icon name="search" />
      <label htmlFor="search" className="visually-hidden">
        Search mail
      </label>
      <input
        id="search"
        ref={(el) => {
          own.current = el;
          setRef(inputRef, el);
        }}
        type="search"
        className="search__input"
        placeholder={placeholder}
        autoComplete="off"
        spellCheck={false}
        aria-autocomplete="list"
        aria-controls={open ? listId : undefined}
        aria-activedescendant={activeIndex >= 0 ? `${listId}-${String(activeIndex)}` : undefined}
        value={value}
        onChange={(e) => {
          change(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => {
          if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') track();
        }}
        onClick={track}
        onFocus={() => {
          setFocused(true);
          track();
        }}
        onBlur={() => {
          setFocused(false);
        }}
      />
      <kbd className="search__key" aria-hidden="true">
        /
      </kbd>
      <button
        ref={helpButton}
        type="button"
        className="icon-btn search__help"
        aria-label="Search tips"
        title="Search tips"
        aria-expanded={help}
        aria-controls={help ? helpId : undefined}
        onClick={() => {
          setHelp((on) => !on);
        }}
      >
        <span aria-hidden="true">?</span>
      </button>
      {open && (
        <div id={listId} role="listbox" className="suggestions search__suggestions" aria-label={`Suggestions for ${context?.field ?? ''}:`}>
          {useful.map((option, index) => (
            // Options are picked with the pointer or the keyboard (handled on the input).
            // eslint-disable-next-line jsx-a11y/click-events-have-key-events
            <div
              key={option.value}
              id={`${listId}-${String(index)}`}
              role="option"
              // Focus stays in the box (aria-activedescendant); options are not tab stops.
              tabIndex={-1}
              aria-selected={index === activeIndex}
              className="suggestion"
              onMouseDown={(e) => {
                e.preventDefault();
              }}
              onClick={() => {
                pick(option);
              }}
            >
              <span className="suggestion__name">{option.label}</span>
              {option.detail && <span className="suggestion__address">{option.detail}</span>}
            </div>
          ))}
        </div>
      )}
      {help && (
        <div
          id={helpId}
          ref={helpRef}
          className="search-help"
          role="dialog"
          aria-label="Search tips"
        >
          <p className="search-help__title">Search tips</p>
          <p className="muted small">Combine as many as you like. Pick an example to add it to your search.</p>
          <dl className="search-help__list">
            {SEARCH_HELP.map((row) => (
              <div key={row.label} className="search-help__row">
                <dt>{row.label}</dt>
                <dd>
                  {row.examples.map((example) => (
                    <button
                      key={example}
                      type="button"
                      className="search-help__example"
                      aria-label={`Add ${example}`}
                      onClick={() => {
                        insertExample(example);
                      }}
                    >
                      <code>{example}</code>
                    </button>
                  ))}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      )}
    </div>
  );
}
