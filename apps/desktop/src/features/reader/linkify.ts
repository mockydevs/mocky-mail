export type TextPart = { kind: 'text'; text: string } | { kind: 'link'; text: string; url: string };

const URL_PATTERN = /\b(?:https?:\/\/|mailto:)[^\s<>"']+/gi;
const TRAILING = /[.,;:!?)\]}]+$/;

/** Split plain text into text and link parts. Only http, https and mailto become links. */
export function linkify(text: string): TextPart[] {
  const parts: TextPart[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const raw = match[0];
    const start = match.index;
    const url = raw.replace(TRAILING, '');
    if (start > last) parts.push({ kind: 'text', text: text.slice(last, start) });
    parts.push({ kind: 'link', text: url, url });
    last = start + url.length;
  }
  if (last < text.length) parts.push({ kind: 'text', text: text.slice(last) });
  return parts;
}
