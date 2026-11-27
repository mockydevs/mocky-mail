/**
 * The rich-text body as HTML strings: plain text made from it, the quoted
 * original under a reply or forward, and the signature block. These work on
 * inert documents from DOMParser (nothing runs, nothing loads), so compose
 * can use them before the editor itself has loaded.
 */

/** Starts a signature in plain text (RFC 3676: dash dash space). The backend looks for it too. */
export const SIGNATURE_SEPARATOR = '-- ';

/** The signature as compose shows it. */
export function signatureBlock(text: string): string {
  return `${SIGNATURE_SEPARATOR}\n${text}`;
}

/** Marks the quoted original under a reply or forward. The backend puts the signature above it. */
export const QUOTE_ATTR = 'data-mocky-quote';
/**
 * Marks the signature block the editor shows (its plain text starts with
 * "-- "). Its value is the signature's id: the backend sends that signature
 * in full in its place, with its formatting and picture.
 */
export const SIGNATURE_ATTR = 'data-mocky-signature';

export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** One paragraph per line, so plain text keeps its shape in the editor. */
export function textToHtml(text: string): string {
  if (text === '') return '<p></p>';
  return text
    .split('\n')
    .map((line) => (line === '' ? '<p></p>' : `<p>${escapeHtml(line)}</p>`))
    .join('');
}

function parse(html: string): HTMLElement {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  for (const hidden of doc.querySelectorAll('script, style, template')) hidden.remove();
  return doc.body;
}

const BLOCKS = new Set([
  'P', 'DIV', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'PRE', 'BLOCKQUOTE', 'UL', 'OL', 'LI', 'TABLE', 'THEAD', 'TBODY',
  'TFOOT', 'TR', 'TD', 'TH', 'HR', 'SECTION', 'ARTICLE', 'HEADER', 'FOOTER', 'ADDRESS', 'FIGURE',
]);

function quoteLine(line: string): string {
  return (line.startsWith('>') ? `>${line}` : `> ${line}`).trimEnd();
}

/** Text of a link: its words, and the address after them when that adds something. */
function linkText(a: HTMLAnchorElement, words: string): string {
  const href = a.getAttribute('href') ?? '';
  const bare = href.replace(/^mailto:/i, '');
  if (href === '' || words.trim() === '' || words.trim() === href || words.trim() === bare) return words || bare;
  return `${words} <${bare}>`;
}

/** Inline content of a node as text (br: a new line). */
function inline(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return (node.textContent ?? '').replace(/[ \t\r\n]+/g, ' ');
  if (!(node instanceof HTMLElement)) return '';
  if (node.tagName === 'BR') return '\n';
  if (node.tagName === 'IMG') return node.getAttribute('alt') ?? '';
  const inner = [...node.childNodes].map(inline).join('');
  return node instanceof HTMLAnchorElement ? linkText(node, inner) : inner;
}

/** The lines of a container whose children may mix inline content and blocks. */
function lines(container: Element): string[] {
  const out: string[] = [];
  let buffer = '';
  let pending = false;
  const flush = () => {
    if (pending) out.push(...buffer.split('\n').map((l) => l.replace(/^ +/, '')));
    buffer = '';
    pending = false;
  };
  for (const child of container.childNodes) {
    if (child instanceof HTMLElement && BLOCKS.has(child.tagName)) {
      flush();
      out.push(...block(child));
    } else {
      const text = inline(child);
      if (text.trim() !== '' || text.includes('\n') || pending) {
        buffer += text;
        pending = true;
      }
    }
  }
  flush();
  return out;
}

function listLines(list: Element, ordered: boolean): string[] {
  const out: string[] = [];
  let n = Number(list.getAttribute('start') ?? '1') || 1;
  for (const item of list.children) {
    if (item.tagName !== 'LI') continue;
    const marker = ordered ? `${String(n++)}. ` : '- ';
    const inner = lines(item);
    const [first = '', ...rest] = inner.length > 0 ? inner : [''];
    out.push(marker + first, ...rest.map((l) => (l === '' ? '' : ' '.repeat(marker.length) + l)));
  }
  return out;
}

function block(el: HTMLElement): string[] {
  switch (el.tagName) {
    case 'BLOCKQUOTE':
      return lines(el).map(quoteLine);
    case 'UL':
      return listLines(el, false);
    case 'OL':
      return listLines(el, true);
    case 'HR':
      return ['----------'];
    case 'TABLE':
    case 'THEAD':
    case 'TBODY':
    case 'TFOOT': {
      const rows: string[] = [];
      for (const child of el.children) {
        if (child instanceof HTMLElement) rows.push(...block(child));
      }
      return rows;
    }
    case 'TR':
      return [
        [...el.children]
          .filter((c) => c.tagName === 'TD' || c.tagName === 'TH')
          .map((cell) => lines(cell).join(' ').trim())
          .join('\t'),
      ];
    default: {
      const inner = lines(el);
      return inner.length > 0 ? inner : [''];
    }
  }
}

/**
 * The plain-text version of the editor's HTML, sent as the text part and
 * kept in `bodyText`: paragraphs on their own lines, "- " and "1. " lists,
 * "> " quotes, links with their address, table cells split by tabs.
 */
export function htmlToPlainText(html: string): string {
  return lines(parse(html))
    .join('\n')
    // No-break spaces (the "-- " of a signature) are plain spaces in text.
    .replace(/[^\S\n\t]/g, ' ')
    .replace(/\n+$/, '');
}

/**
 * The original under a reply or forward, in the marked block the backend
 * puts the signature above: the header line ("On …, … wrote:" or the
 * forwarded block), then the original's HTML (already sanitized by the
 * backend) or its text, in a blockquote for a reply.
 */
export function quotedOriginal(
  headerLines: readonly string[],
  original: { html: string | null; text: string | null },
  asQuote: boolean,
): string {
  const header = `<p>${headerLines.map(escapeHtml).join('<br>')}</p>`;
  const body = original.html ?? textToHtml(original.text ?? '');
  return `<div ${QUOTE_ATTR}="">${header}${asQuote ? `<blockquote>${body}</blockquote>` : body}</div>`;
}

/** The signature as the editor shows it: "-- " and the words, one paragraph per line. */
export function signatureHtml(text: string, id: string | null = null): string {
  const words = text.split('\n').map((line) => (line === '' ? '<p></p>' : `<p>${escapeHtml(line)}</p>`));
  // A no-break space, so the "-- " separator keeps its space through the editor.
  return `<div ${SIGNATURE_ATTR}="${escapeHtml(id ?? '')}"><p>--&nbsp;</p>${words.join('')}</div>`;
}

function serialize(body: HTMLElement): string {
  return body.innerHTML;
}

/** Whether the body still has this signature's block, unedited. */
export function hasSignature(html: string, text: string): boolean {
  return htmlToPlainText(html).includes(signatureBlock(text));
}

/**
 * Put signature `next` in place of `previous` (null: none) in the body. It
 * goes under where the user types, above the quoted original. A block the
 * user edited is left alone, and no second one is added.
 */
export function swapSignatureHtml(html: string, previous: string | null, next: string | null, nextId: string | null = null): string {
  if (previous === next) return html;
  const body = parse(html);
  const blocks = [...body.querySelectorAll(`[${SIGNATURE_ATTR}]`)];
  if (previous) {
    const old = blocks.find((b) => htmlToPlainText(b.innerHTML) === signatureBlock(previous));
    if (!old) return html;
    if (next) {
      const holder = body.ownerDocument.createElement('div');
      holder.innerHTML = signatureHtml(next, nextId);
      if (holder.firstElementChild) old.replaceWith(holder.firstElementChild);
    } else {
      old.remove();
    }
    return serialize(body);
  }
  if (!next) return html;
  if (blocks.some((b) => htmlToPlainText(b.innerHTML) === signatureBlock(next))) return html;
  const holder = body.ownerDocument.createElement('div');
  holder.innerHTML = signatureHtml(next, nextId);
  const signature = holder.firstElementChild;
  if (!signature) return html;
  const quote = [...body.children].find((c) => c.hasAttribute(QUOTE_ATTR));
  if (quote) body.insertBefore(signature, quote);
  else body.append(signature);
  // Somewhere to type above it.
  if (body.firstElementChild === signature) body.insertBefore(body.ownerDocument.createElement('p'), signature);
  return serialize(body);
}
