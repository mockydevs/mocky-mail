/**
 * Phone layout for HTML email.
 *
 * Most marketing email is laid out for a 600px desktop window, with its own
 * phone rules in a `<style>` block. Our sanitizer removes `<style>` blocks and
 * classes, so on a phone those emails come out as boxes inside boxes: the
 * email's gutters and card borders stack on top of each other and a
 * two-column row is squeezed until words break letter by letter.
 *
 * On a phone we therefore flatten the email after it loads: side padding,
 * side margins and side borders of layout containers go, so text and images
 * sit directly on the page, and side-by-side columns that are too narrow to
 * read are stacked. Small coloured boxes (buttons, badges) keep their padding.
 * This only rearranges the already sanitized email; it adds no content.
 */

/** Base rules added to the email document in the phone layout. */
export const PHONE_STYLE =
  'body{padding:12px 16px}' +
  'table,td,th,div,center{max-width:100%!important;box-sizing:border-box}' +
  'img{max-width:100%!important;height:auto!important}';

const CONTAINERS = 'table,tbody,tr,td,th,div,center';

/** Narrower than this share of the page, a cell holding text or a picture is stacked. */
const CRAMPED_SHARE = 0.55;

/** A cell this narrow with no text is an icon next to a label; that row stays as it is. */
const ICON_WIDTH = 72;

function hasBackground(style: CSSStyleDeclaration): boolean {
  const colour = style.backgroundColor;
  return colour !== '' && colour !== 'transparent' && colour !== 'rgba(0, 0, 0, 0)';
}

function setImportant(element: HTMLElement, property: string, value: string) {
  element.style.setProperty(property, value, 'important');
}

/** Flatten an email's nested boxes and stack cramped columns. */
export function fitToPhone(doc: Document): void {
  const view = doc.defaultView;
  const body = doc.body as HTMLElement | null;
  if (!view || !body) return;
  const pageWidth = doc.documentElement.clientWidth;

  // The email's outer background becomes the page's, so no band of another
  // colour frames it at the screen edges.
  for (const element of body.querySelectorAll<HTMLElement>(CONTAINERS)) {
    const style = view.getComputedStyle(element);
    if (element.getBoundingClientRect().width < pageWidth * 0.8) continue;
    if (hasBackground(style)) {
      setImportant(doc.documentElement, 'background-color', style.backgroundColor);
      setImportant(body, 'background-color', style.backgroundColor);
      break;
    }
  }

  for (const element of body.querySelectorAll<HTMLElement>(CONTAINERS)) {
    const style = view.getComputedStyle(element);
    const width = element.getBoundingClientRect().width;
    // Buttons and badges: a coloured box that doesn't span the page keeps its padding.
    const button = hasBackground(style) && width > 0 && width < pageWidth * 0.7;
    if (!button) {
      if (parseFloat(style.paddingLeft) > 0) setImportant(element, 'padding-left', '0');
      if (parseFloat(style.paddingRight) > 0) setImportant(element, 'padding-right', '0');
    }
    if (parseFloat(style.marginLeft) > 0 || parseFloat(style.marginRight) > 0) {
      setImportant(element, 'margin-left', '0');
      setImportant(element, 'margin-right', '0');
    }
    if (!button && (parseFloat(style.borderLeftWidth) > 0 || parseFloat(style.borderRightWidth) > 0)) {
      setImportant(element, 'border-left-width', '0');
      setImportant(element, 'border-right-width', '0');
      setImportant(element, 'border-radius', '0');
    }
    const declared = Number.parseFloat(element.getAttribute('width') ?? '');
    if (element.tagName === 'TABLE' && declared > pageWidth) setImportant(element, 'width', '100%');
  }

  for (const row of body.querySelectorAll('tr')) {
    const cells = [...row.children].filter(
      (cell): cell is HTMLElement => cell.tagName === 'TD' || cell.tagName === 'TH',
    );
    if (cells.length < 2) continue;
    const filled = cells.filter(
      (cell) =>
        cell.getBoundingClientRect().width > 0 &&
        (cell.textContent.trim() !== '' || cell.querySelector('img, table') !== null),
    );
    if (filled.length < 2) continue;
    const isIcon = (cell: HTMLElement) =>
      cell.getBoundingClientRect().width < ICON_WIDTH && cell.textContent.trim() === '';
    if (filled.some(isIcon)) continue;
    const cramped = filled.some((cell) => {
      if (cell.getBoundingClientRect().width >= pageWidth * CRAMPED_SHARE) return false;
      const picture = cell.querySelector('img');
      return cell.textContent.trim().length > 40 || (picture !== null && picture.getBoundingClientRect().width > 120);
    });
    if (!cramped) continue;
    for (const cell of cells) {
      setImportant(cell, 'display', 'block');
      setImportant(cell, 'width', '100%');
    }
  }
}
