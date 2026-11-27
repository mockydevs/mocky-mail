import { HAS_VALUES, IS_VALUES, isOr, tokenize } from '../../features/search/query';
import type { Folder, ParsedSearch, SearchTerm } from '../types';
import type { MockMessage } from './mockMailApi';

const EXAMPLES: Readonly<Record<string, string>> = {
  from: 'from:wanjiru',
  to: 'to:peter',
  cc: 'cc:amina',
  subject: 'subject:invoice',
  body: 'body:receipt',
  category: 'category:Clients',
  in: 'in:archive',
};

/** `500k`, `2m`, `1.5mb`, or bytes: the size in bytes, or null. */
export function parseSize(text: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*(b|k|kb|m|mb|g|gb)?$/i.exec(text.trim());
  if (!m) return null;
  const unit = (m[2] ?? 'b').toLowerCase()[0];
  const scale = unit === 'k' ? 1024 : unit === 'm' ? 1024 ** 2 : unit === 'g' ? 1024 ** 3 : 1;
  return Math.round(Number(m[1]) * scale);
}

/** Start of a day (local time) as Unix seconds: `YYYY-MM-DD`, `today` or `yesterday`; null if not a date. */
export function parseDay(text: string, now: number): number | null {
  const value = text.trim().toLowerCase();
  if (value === 'today' || value === 'yesterday') {
    const day = new Date(now * 1000);
    day.setHours(0, 0, 0, 0);
    if (value === 'yesterday') day.setDate(day.getDate() - 1);
    return Math.floor(day.getTime() / 1000);
  }
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const [year, month, date] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const day = new Date(year, month - 1, date);
  if (day.getFullYear() !== year || day.getMonth() !== month - 1 || day.getDate() !== date) return null;
  return Math.floor(day.getTime() / 1000);
}

/** Read a search box query as the backend does: terms for chips, plain-language errors. */
export function parseSearchQuery(query: string, now: number): ParsedSearch {
  const terms: SearchTerm[] = [];
  const errors: string[] = [];
  /** An OR came after the last term: join it to the next one. */
  let joinNext = false;
  const add = (term: SearchTerm) => {
    const last = terms.at(-1);
    if (joinNext && last) last.orWithNext = true;
    joinNext = false;
    terms.push(term);
  };
  for (const token of tokenize(query)) {
    if (isOr(token)) {
      joinNext = terms.length > 0;
      continue;
    }
    const value = token.value.trim();
    const { negated } = token;
    const field = token.field;
    if (field === null) {
      if (value) add({ field: 'text', value, negated });
      continue;
    }
    if (!value) {
      errors.push(`${field}: needs something after it, like ${EXAMPLES[field] ?? `${field}:…`}`);
      continue;
    }
    switch (field) {
      case 'is': {
        const v = value.toLowerCase();
        if ((IS_VALUES as readonly string[]).includes(v)) add({ field, value: v, negated });
        else errors.push('is: can be unread, read, flagged, pinned or answered');
        break;
      }
      case 'has': {
        const v = value.toLowerCase().replace(/s$/, '');
        if ((HAS_VALUES as readonly string[]).includes(v)) add({ field, value: v, negated });
        else errors.push('has: only knows has:attachment');
        break;
      }
      case 'before':
      case 'after':
        if (parseDay(value, now) === null) errors.push(`${field}: needs a date like 2026-10-01, today or yesterday`);
        else add({ field, value: value.toLowerCase(), negated });
        break;
      case 'larger':
      case 'smaller':
        if (parseSize(value) === null) errors.push(`${field}: needs a size like 500k or 2m`);
        else add({ field, value: value.toLowerCase(), negated });
        break;
      case 'from':
      case 'to':
      case 'cc':
      case 'subject':
      case 'body':
      case 'category':
      case 'in':
        add({ field, value, negated });
        break;
    }
  }
  return { terms, errors };
}

export interface SearchContext {
  now: number;
  folderOf: (id: string) => Omit<Folder, 'unread' | 'total'> | undefined;
}

const has = (haystack: string, needle: string) => haystack.toLowerCase().includes(needle.toLowerCase());
const people = (list: readonly { name: string | null; address: string }[]) =>
  list.map((p) => `${p.name ?? ''} ${p.address}`).join(' ');

function matchesTerm(m: MockMessage, term: SearchTerm, ctx: SearchContext): boolean {
  const v = term.value;
  switch (term.field) {
    case 'text':
      return has(
        [m.subject, m.preview, m.text ?? '', m.from?.name ?? '', m.from?.address ?? '', ...m.to.map((t) => t.address)].join(' '),
        v,
      );
    case 'from':
      return has(m.from ? people([m.from]) : '', v);
    case 'to':
      return has(people(m.to), v);
    case 'cc':
      return has(people(m.cc), v);
    case 'subject':
      return has(m.subject, v);
    case 'body':
      return has(`${m.preview} ${m.text ?? ''} ${(m.html ?? '').replace(/<[^>]*>/g, ' ')}`, v);
    case 'has':
      return m.hasAttachments;
    case 'is':
      switch (v) {
        case 'unread':
          return m.unread;
        case 'read':
          return !m.unread;
        case 'flagged':
          return m.flagged;
        case 'pinned':
          return m.pinned;
        default:
          return m.answered;
      }
    case 'category':
      return m.categories.some((c) => c.toLowerCase() === v.toLowerCase());
    case 'before':
      return (m.date ?? 0) < (parseDay(v, ctx.now) ?? 0);
    case 'after':
      return (m.date ?? 0) >= (parseDay(v, ctx.now) ?? 0);
    case 'in': {
      const folder = ctx.folderOf(m.folderId);
      if (!folder) return false;
      const want = v.toLowerCase();
      const role = want === 'spam' ? 'junk' : want;
      return folder.role === role || folder.name.toLowerCase() === want || folder.path.toLowerCase() === want;
    }
    case 'larger':
      return m.size > (parseSize(v) ?? Infinity);
    case 'smaller':
      return m.size < (parseSize(v) ?? 0);
  }
}

/** Every term must match (a negated one must not); terms joined by OR need only one of them. */
export function matchesSearch(m: MockMessage, terms: readonly SearchTerm[], ctx: SearchContext): boolean {
  const groups: SearchTerm[][] = [];
  let open = false;
  for (const term of terms) {
    if (open) groups.at(-1)?.push(term);
    else groups.push([term]);
    open = term.orWithNext === true;
  }
  return groups.every((group) => group.some((t) => matchesTerm(m, t, ctx) !== t.negated));
}

/** A search naming a folder (`in:`, not negated) looks in the whole account. */
export function namesFolder(terms: readonly SearchTerm[]): boolean {
  return terms.some((t) => t.field === 'in' && !t.negated);
}
