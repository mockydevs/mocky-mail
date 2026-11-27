import type { SearchScope, SearchTerm } from '../../api/types';

/** The operators of the search language, without their colon. */
export const OPERATORS = [
  'from',
  'to',
  'cc',
  'subject',
  'body',
  'has',
  'is',
  'category',
  'before',
  'after',
  'in',
  'larger',
  'smaller',
] as const;

export type Operator = (typeof OPERATORS)[number];

export function isOperator(name: string): name is Operator {
  return (OPERATORS as readonly string[]).includes(name);
}

/** Values `is:` takes. */
export const IS_VALUES = ['unread', 'read', 'flagged', 'pinned', 'answered'] as const;
/** Values `has:` takes. */
export const HAS_VALUES = ['attachment'] as const;

/** One word, phrase or `operator:value` as typed, with where it sits in the query. */
export interface Token {
  /** Index of the first character (the `-` of a negation). */
  start: number;
  /** Index after the last character. */
  end: number;
  negated: boolean;
  /** null for plain words and phrases. */
  field: Operator | null;
  /** The value, without quotes. */
  value: string;
  /** Where the value starts (after `operator:` and any opening quote). */
  valueStart: number;
}

function unquote(text: string): string {
  return text.replace(/"/g, '');
}

/**
 * Split a query the way the search language reads it: spaces separate
 * terms except inside "quotes", `-` in front negates, and `word:` is an
 * operator only when it is one of OPERATORS (so a URL stays plain text).
 */
export function tokenize(query: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < query.length) {
    if (/\s/.test(query[i] ?? '')) {
      i += 1;
      continue;
    }
    const start = i;
    let quoted = false;
    while (i < query.length && (quoted || !/\s/.test(query[i] ?? ''))) {
      if (query[i] === '"') quoted = !quoted;
      i += 1;
    }
    let raw = query.slice(start, i);
    let offset = start;
    let negated = false;
    if (raw.startsWith('-') && raw.length > 1) {
      negated = true;
      raw = raw.slice(1);
      offset += 1;
    }
    const colon = raw.indexOf(':');
    const name = colon > 0 ? raw.slice(0, colon).toLowerCase() : '';
    if (colon > 0 && isOperator(name)) {
      const rest = raw.slice(colon + 1);
      const valueStart = offset + colon + 1 + (rest.startsWith('"') ? 1 : 0);
      tokens.push({ start, end: i, negated, field: name, value: unquote(rest), valueStart });
    } else {
      tokens.push({ start, end: i, negated, field: null, value: unquote(raw), valueStart: offset + (raw.startsWith('"') ? 1 : 0) });
    }
  }
  return tokens;
}

/** Quote a value that has spaces, so it stays one term. */
export function quoteValue(value: string): string {
  return /\s/.test(value) ? `"${value.replace(/"/g, '')}"` : value;
}

/** `OR` between two terms (not a term itself). */
export function isOr(token: Token): boolean {
  return token.field === null && !token.negated && token.value === 'OR';
}

/** Take a part out of a query and tidy what it leaves: spaces, and an OR left with nothing on one side. */
function cut(query: string, token: Token): string {
  const words = `${query.slice(0, token.start)} ${query.slice(token.end)}`.trim().split(/\s+/);
  const kept = words.filter((w, i) => w !== 'OR' || (i > 0 && i < words.length - 1 && words[i - 1] !== 'OR'));
  return kept.join(' ');
}

const same = (a: string, b: string) => a.trim().toLocaleLowerCase() === b.trim().toLocaleLowerCase();

/**
 * The query without `term` (one chip). Found by what it says; failing that
 * (the backend may tidy a value), by its place among terms of its kind.
 */
export function removeTerm(query: string, terms: readonly SearchTerm[], term: SearchTerm): string {
  const tokens = tokenize(query).filter((t) => (t.value.trim() !== '' || t.field !== null) && !isOr(t));
  const kind = (t: Token) => t.field ?? 'text';
  const exact = tokens.find((t) => kind(t) === term.field && t.negated === term.negated && same(t.value, term.value));
  if (exact) return cut(query, exact);
  const sameKind = tokens.filter((t) => kind(t) === term.field && t.negated === term.negated);
  const place = terms.filter((t) => t.field === term.field && t.negated === term.negated).indexOf(term);
  const found = sameKind[place] ?? (sameKind.length === 1 ? sameKind[0] : undefined);
  return found ? cut(query, found) : query;
}

/** Add a piece (a help example, say) to the end of the query, ready for more typing. */
export function appendTerm(query: string, piece: string): string {
  const base = query.trim();
  return `${base ? `${base} ` : ''}${piece} `;
}

/** The operator being typed where the caret is, if any: what to suggest values for. */
export interface OperatorContext {
  field: Operator;
  /** What has been typed of the value so far. */
  partial: string;
  token: Token;
}

export function operatorAt(query: string, caret: number): OperatorContext | null {
  const token = tokenize(query).find((t) => t.start <= caret && caret <= t.end);
  if (!token?.field || caret < token.valueStart) return null;
  return { field: token.field, partial: unquote(query.slice(token.valueStart, caret)), token };
}

/** Put a chosen suggestion in as the operator's value; returns the new query and where the caret goes. */
export function applySuggestion(query: string, context: OperatorContext, value: string): { query: string; caret: number } {
  const { token } = context;
  const prefix = query.slice(0, token.valueStart - (query[token.valueStart - 1] === '"' ? 1 : 0));
  const after = query.slice(token.end).replace(/^\s+/, '');
  const inserted = `${prefix}${quoteValue(value)} `;
  return { query: `${inserted}${after}`, caret: inserted.length };
}

/** How a chip reads: `from: boss`, `not is: read`, `"big deal"`. */
export function termLabel(term: SearchTerm): string {
  const value = term.field === 'text' ? (/\s/.test(term.value) ? `“${term.value}”` : term.value) : `${term.field}: ${term.value}`;
  return term.negated ? `not ${value}` : value;
}

export const SCOPE_LABELS: Readonly<Record<SearchScope, string>> = {
  folder: 'This folder',
  subfolders: 'Including subfolders',
  account: 'This account',
  all: 'All accounts',
};

/** The help popover: what each operator does, with examples to insert. */
export const SEARCH_HELP: readonly { label: string; examples: readonly string[] }[] = [
  { label: 'Words anywhere; quotes keep a phrase together', examples: ['invoice', '"site visit"'] },
  { label: 'From, to or copied to someone', examples: ['from:wanjiru', 'to:peter@client.example', 'cc:amina'] },
  { label: 'Words in the subject or the message', examples: ['subject:quotation', 'body:receipt'] },
  { label: 'With an attachment', examples: ['has:attachment'] },
  { label: 'Unread, read, flagged, pinned or replied to', examples: ['is:unread', 'is:flagged', 'is:answered'] },
  { label: 'With a category', examples: ['category:Clients'] },
  { label: 'Before a day, or on and after it', examples: ['after:2026-10-01', 'before:today', 'after:yesterday'] },
  { label: 'In a folder', examples: ['in:inbox', 'in:archive'] },
  { label: 'By size (k for KB, m for MB)', examples: ['larger:500k', 'smaller:2m'] },
  { label: 'Leave out what matches with a minus', examples: ['-is:read', '-from:noreply'] },
  { label: 'Either of two: OR between them', examples: ['from:amina OR from:peter'] },
];
