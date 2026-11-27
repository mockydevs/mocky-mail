import { useEffect, useId, useRef, useState } from 'react';

import type { MailApi, ParsedSearch, SearchScope, SearchTerm } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { removeTerm, SCOPE_LABELS, termLabel } from './query';

/**
 * The settled search, read back by the backend for chips and errors. The
 * last answer stays until the next arrives, so chips don't flicker while
 * typing; an empty search has none.
 */
export function useParsedSearch(api: MailApi, query: string): ParsedSearch | null {
  const [parsed, setParsed] = useState<(ParsedSearch & { query: string }) | null>(null);
  const ticket = useRef(0);
  const text = query.trim();
  useEffect(() => {
    const mine = ++ticket.current;
    if (text === '') return;
    api.parseSearch(text).then(
      (result) => {
        if (mine === ticket.current) setParsed({ ...result, query: text });
      },
      () => undefined,
    );
  }, [api, text]);
  return text === '' || !parsed ? null : parsed;
}

interface Props {
  /** What is in the search box now. */
  query: string;
  parsed: ParsedSearch | null;
  scope: SearchScope;
  /** The scopes that make sense for the open view, in order. */
  scopes: readonly SearchScope[];
  /** Labels to use instead of the usual ones (the unified views read differently). */
  labels?: Partial<Record<SearchScope, string>>;
  onScope: (scope: SearchScope) => void;
  onQuery: (query: string) => void;
}

/** Under the top bar while searching: where to look, the search as chips, and what it didn't understand. */
export function SearchStrip({ query, parsed, scope, scopes, labels, onScope, onQuery }: Props) {
  const id = useId();
  const remove = (term: SearchTerm) => {
    if (parsed) onQuery(removeTerm(query, parsed.terms, term));
  };
  return (
    <section className="search-strip" aria-label="Search options">
      <label className="search-strip__scope" htmlFor={`${id}-scope`}>
        <span className="search-strip__label">Search in</span>
        <select
          id={`${id}-scope`}
          className="field field--compact"
          value={scope}
          onChange={(e) => {
            const next = scopes.find((s) => s === e.target.value);
            if (next) onScope(next);
          }}
        >
          {scopes.map((s) => (
            <option key={s} value={s}>
              {labels?.[s] ?? SCOPE_LABELS[s]}
            </option>
          ))}
        </select>
      </label>
      {parsed && parsed.terms.length > 0 && (
        <ul className="search-strip__terms" aria-label="Search terms">
          {parsed.terms.map((term, index) => {
            const label = termLabel(term);
            return (
              <li key={`${String(index)}-${term.field}-${term.value}`} className="search-strip__term">
                <span className={`search-chip${term.negated ? ' search-chip--not' : ''}`}>
                  <span className="search-chip__text">{label}</span>
                  <button
                    type="button"
                    className="chip__remove"
                    aria-label={`Remove ${label}`}
                    title="Remove from the search"
                    onClick={() => {
                      remove(term);
                    }}
                  >
                    <Icon name="close" size={12} />
                  </button>
                </span>
                {term.orWithNext && index < parsed.terms.length - 1 && <span className="search-strip__or"> or</span>}
              </li>
            );
          })}
        </ul>
      )}
      {parsed && parsed.errors.length > 0 && (
        <div className="search-strip__errors" role="status">
          {parsed.errors.map((error) => (
            <p key={error} className="search-strip__error">
              {error}
            </p>
          ))}
        </div>
      )}
    </section>
  );
}
