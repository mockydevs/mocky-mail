/**
 * Phone layout for HTML email.
 *
 * Most marketing email is laid out for a 600px desktop window, with its own
 * phone rules in a `<style>` block. Our sanitizer removes `<style>` blocks and
 * classes, so on a phone those emails come out as boxes inside boxes: the
 * email's gutters and card borders stack on top of each other, fixed widths
 * push content off the screen, and columns are squeezed until words break
 * letter by letter.
 *
 * On a phone we therefore rearrange the email after it loads:
 * 1. Anything wider than the screen is held to it (fixed widths, min-widths,
 *    unwrapped `<pre>` text).
 * 2. Side padding, margins and borders of layout boxes go, so text and images
 *    sit directly on the page. Buttons and badges keep theirs; coloured cards
 *    (footers, highlighted panels) span the screen with a little padding; the
 *    page takes the email's outer background.
 * 3. Columns too narrow to read are stacked, whether built from table cells,
 *    floated tables or inline blocks. Icon-and-label rows stay as they are.
 * 4. Text smaller than 11px is raised to 11px.
 * This only rearranges the already sanitized email; it adds no content.
 */

/** Base rules added to the email document in the phone layout. */
export const PHONE_STYLE =
  'body{padding:12px 16px}' +
  'table,td,th,div,center{max-width:100%!important;min-width:0!important;box-sizing:border-box}' +
  'img{max-width:100%!important;height:auto!important}' +
  'pre,code{white-space:pre-wrap!important;overflow-wrap:anywhere!important}' +
  'hr{max-width:100%!important;box-sizing:border-box}';

const CONTAINERS = 'table,tbody,tr,td,th,div,center,hr';

/** Elements that carry readable text. */
const TEXT = 'p,td,th,div,span,a,h1,h2,h3,h4,h5,h6,li,font,strong,b,em,i';

/** A column narrower than this share of the screen, holding a sentence, is stacked. */
const NARROW_SHARE = 0.55;

/** At least this many characters of its own text makes a block worth reading at full width. */
const SENTENCE = 25;

/** A cell this narrow with no text is an icon next to a label; that row stays as it is. */
const ICON_WIDTH = 72;

/** Side padding left on a coloured card, so its text doesn't touch the card's edge. */
const CARD_PADDING = 16;

/** Smallest text size on a phone. */
const MIN_FONT = 11;

function hasBackground(style: CSSStyleDeclaration): boolean {
  const colour = style.backgroundColor;
  return colour !== '' && colour !== 'transparent' && colour !== 'rgba(0, 0, 0, 0)';
}

/** A button or badge: a coloured box holding only a short label, no pictures or tables. */
function isButton(element: HTMLElement): boolean {
  return element.textContent.trim().length <= 40 && element.querySelector('table, img') === null;
}

function setImportant(element: HTMLElement, property: string, value: string) {
  element.style.setProperty(property, value, 'important');
}

function widthOf(element: Element): number {
  return element.getBoundingClientRect().width;
}

/** Whether the element reaches past the right edge of the screen. */
function overflows(element: Element, pageWidth: number): boolean {
  return element.getBoundingClientRect().right > pageWidth + 1;
}

/** The element's own text, not counting its children's. */
function ownText(element: Element): string {
  let text = '';
  for (const node of element.childNodes) if (node.nodeType === 3) text += node.textContent ?? '';
  return text.trim();
}

/** 1. Hold everything to the screen width. */
function fitWidths(body: HTMLElement, view: Window, pageWidth: number) {
  // A picture in a table cell ignores max-width when the table sizes itself,
  // so a wide one gets an explicit width.
  for (const image of body.querySelectorAll<HTMLElement>('img')) {
    const declared = Number.parseFloat(image.getAttribute('width') ?? '');
    if (overflows(image, pageWidth) || declared > pageWidth) setImportant(image, 'width', '100%');
  }
  for (const element of body.querySelectorAll<HTMLElement>(CONTAINERS)) {
    const declared = Number.parseFloat(element.getAttribute('width') ?? '');
    if (!overflows(element, pageWidth) && !(declared > pageWidth)) continue;
    setImportant(element, 'width', element.tagName === 'TABLE' ? '100%' : 'auto');
  }
  // Text kept on one line that spills out of its box wraps instead.
  for (const element of body.querySelectorAll<HTMLElement>(TEXT)) {
    if (element.scrollWidth > element.clientWidth + 1 && view.getComputedStyle(element).whiteSpace === 'nowrap') {
      setImportant(element, 'white-space', 'normal');
    }
  }
  // Text kept on one line (dates, amounts, links) mustn't hold a row wider than the screen.
  for (const row of body.querySelectorAll<HTMLElement>('tr')) {
    if (!overflows(row, pageWidth)) continue;
    for (const cell of row.querySelectorAll<HTMLElement>('td, th')) {
      if (view.getComputedStyle(cell).whiteSpace === 'nowrap') setImportant(cell, 'white-space', 'normal');
    }
  }
}

/** A real data table that still can't fit scrolls sideways in its own box, not the whole page. */
function scrollWideTables(body: HTMLElement, pageWidth: number) {
  // First try fixed layout: the table takes the screen's width and its
  // content (a 600px banner, say) can no longer push it wider.
  for (const table of body.querySelectorAll<HTMLElement>('table')) {
    if (overflows(table, pageWidth)) setImportant(table, 'table-layout', 'fixed');
  }
  for (const table of body.querySelectorAll<HTMLElement>('table')) {
    if (!overflows(table, pageWidth)) continue;
    // Only the outermost wide table; tables inside it scroll with it.
    const outer = table.parentElement?.closest('table');
    if (outer && overflows(outer, pageWidth)) continue;
    const holder = table.parentElement;
    if (!holder) continue;
    setImportant(holder, 'overflow-x', 'auto');
    setImportant(holder, 'max-width', '100%');
  }
}

/** 2. Take away the nested boxes; returns nothing, changes styles in place. */
function flatten(doc: Document, body: HTMLElement, view: Window, pageWidth: number) {
  // The email's outer background becomes the page's, so no band of another
  // colour frames it at the screen edges.
  // Only a box that wraps most of the email counts: a coloured header bar
  // would otherwise paint the whole page and leave the body text unreadable.
  let page = view.getComputedStyle(body).backgroundColor;
  const emailHeight = body.scrollHeight;
  for (const element of body.querySelectorAll<HTMLElement>(CONTAINERS)) {
    const box = element.getBoundingClientRect();
    if (box.width < pageWidth * 0.8 || box.height < emailHeight * 0.8) continue;
    const style = view.getComputedStyle(element);
    if (hasBackground(style)) {
      page = style.backgroundColor;
      setImportant(doc.documentElement, 'background-color', page);
      setImportant(body, 'background-color', page);
      break;
    }
  }

  for (const element of body.querySelectorAll<HTMLElement>(CONTAINERS)) {
    const style = view.getComputedStyle(element);
    const coloured = hasBackground(style) && style.backgroundColor !== page;
    // Buttons and badges keep their size and padding.
    const button = coloured && isButton(element);
    // A coloured card (a footer, a highlighted panel) spans the screen with a
    // little padding, instead of shrinking to its shortest line.
    const card = coloured && !button;
    if (card) {
      setImportant(element, 'width', '100%');
      const table = element.closest('table');
      if (table) setImportant(table, 'width', '100%');
      for (const side of ['padding-left', 'padding-right'] as const) {
        if (parseFloat(style.getPropertyValue(side)) > CARD_PADDING) setImportant(element, side, `${String(CARD_PADDING)}px`);
      }
    } else if (!button) {
      if (parseFloat(style.paddingLeft) > 0) setImportant(element, 'padding-left', '0');
      if (parseFloat(style.paddingRight) > 0) setImportant(element, 'padding-right', '0');
    }
    if (parseFloat(style.marginLeft) > 0 || parseFloat(style.marginRight) > 0) {
      setImportant(element, 'margin-left', '0');
      setImportant(element, 'margin-right', '0');
    }
    if (!button && !card && (parseFloat(style.borderLeftWidth) > 0 || parseFloat(style.borderRightWidth) > 0)) {
      setImportant(element, 'border-left-width', '0');
      setImportant(element, 'border-right-width', '0');
      setImportant(element, 'border-radius', '0');
    }
  }
}

function isIcon(cell: Element): boolean {
  return widthOf(cell) < ICON_WIDTH && cell.textContent.trim() === '';
}

/** Show a column at full width, one under the other. */
function stack(column: HTMLElement) {
  setImportant(column, 'display', 'block');
  setImportant(column, 'width', '100%');
  setImportant(column, 'float', 'none');
}

/** Stack every cell of a row, unless it is an icon beside its label. */
function stackRow(row: Element): boolean {
  const cells = [...row.children].filter((cell): cell is HTMLElement => cell.tagName === 'TD' || cell.tagName === 'TH');
  const filled = cells.filter(
    (cell) => widthOf(cell) > 0 && (cell.textContent.trim() !== '' || cell.querySelector('img, table') !== null),
  );
  if (filled.length < 2 || filled.some(isIcon)) return false;
  // A cell can only leave its column if the table stops being a grid: the
  // table and its rows become plain blocks. Rows that aren't stacked keep
  // their cells side by side, each row as its own small grid.
  const table = row.closest('table');
  if (table) {
    const parts = table.querySelectorAll<HTMLElement>(
      ':scope > tbody, :scope > thead, :scope > tfoot, :scope > tr, :scope > tbody > tr, :scope > thead > tr, :scope > tfoot > tr',
    );
    for (const part of [table, ...parts]) stack(part);
  }
  for (const cell of cells) stack(cell);
  return true;
}

/** 3. Stack columns too narrow to read. */
function stackColumns(body: HTMLElement, pageWidth: number) {
  const narrow = pageWidth * NARROW_SHARE;

  // Table rows whose cells hold a sentence or a large picture squeezed side by side.
  for (const row of body.querySelectorAll('tr')) {
    const cramped = [...row.children].some((cell) => {
      if (cell.tagName !== 'TD' && cell.tagName !== 'TH') return false;
      if (widthOf(cell) >= narrow || isIcon(cell)) return false;
      const picture = cell.querySelector('img');
      return cell.textContent.trim().length > 40 || (picture !== null && widthOf(picture) > 120);
    });
    if (cramped) stackRow(row);
  }

  // Any other narrow column holding a sentence: floated tables, inline blocks,
  // or a cell inside a narrow nested table. Find the outermost narrow box
  // around the text and stack it with its neighbours.
  for (const element of body.querySelectorAll<HTMLElement>(TEXT)) {
    if (ownText(element).length < SENTENCE) continue;
    const width = widthOf(element);
    if (width === 0 || width >= narrow) continue;
    let column: HTMLElement = element;
    while (column.parentElement && column.parentElement !== body && widthOf(column.parentElement) < narrow) {
      column = column.parentElement;
    }
    if (column.tagName === 'TD' || column.tagName === 'TH') {
      if (column.parentElement && stackRow(column.parentElement)) continue;
      stack(column);
      continue;
    }
    // The email lives in its own frame, so its elements aren't instances of
    // this window's HTMLElement: select them as HTML elements instead.
    const neighbours = column.parentElement ? column.parentElement.querySelectorAll<HTMLElement>(':scope > *') : [column];
    for (const neighbour of neighbours) {
      if (widthOf(neighbour) < narrow && !isIcon(neighbour)) stack(neighbour);
    }
  }
}

/** 4. Raise text too small to read on a phone. */
function readableText(body: HTMLElement, view: Window) {
  for (const element of body.querySelectorAll<HTMLElement>(TEXT)) {
    if (ownText(element) === '') continue;
    if (parseFloat(view.getComputedStyle(element).fontSize) < MIN_FONT) setImportant(element, 'font-size', `${String(MIN_FONT)}px`);
  }
}

/** Rearrange a loaded email for a phone screen. */
export function fitToPhone(doc: Document): void {
  const view = doc.defaultView;
  const body = doc.body as HTMLElement | null;
  if (!view || !body) return;
  const pageWidth = doc.documentElement.clientWidth;
  fitWidths(body, view, pageWidth);
  flatten(doc, body, view, pageWidth);
  stackColumns(body, pageWidth);
  readableText(body, view);
  // Stacking can free a row that was holding a table wide; check widths again.
  fitWidths(body, view, pageWidth);
  scrollWideTables(body, pageWidth);
}
