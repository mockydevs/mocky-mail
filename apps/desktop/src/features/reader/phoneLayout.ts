/**
 * Phone layout for HTML email, the way Gmail and Outlook do it.
 *
 * Designed email carries its own phone layout: `@media` rules in its
 * stylesheet, percentage widths, `max-width` cards. The sanitizer keeps all
 * of that, so most email lays itself out for the screen with no help.
 *
 * What is left is email built only for a 600px desktop window. Rearranging
 * it (stripping padding, stacking columns, forcing widths) breaks tables
 * such as receipts, whose columns belong side by side. Instead the whole
 * email is shown as designed and scaled down until it fits the screen, as
 * the big mail apps do. Nothing is moved, so nothing gets cropped.
 */

/** Base rules added to the email document in the phone layout. */
export const PHONE_STYLE =
  'body{padding:12px 16px}' +
  'img{max-width:100%!important;height:auto!important}' +
  'pre,code{white-space:pre-wrap!important;overflow-wrap:break-word!important}';

/**
 * Smallest scale an email is shrunk to. Below this its text is too small to
 * read even zoomed in, so it scrolls sideways instead.
 */
export const MIN_SCALE = 0.5;

/** How much the email must shrink to fit, or 1 when it already fits. */
export function fitScale(contentWidth: number, pageWidth: number): number {
  if (pageWidth <= 0 || contentWidth <= pageWidth + 1) return 1;
  return Math.max(MIN_SCALE, pageWidth / contentWidth);
}

/**
 * Whether the email is a designed one that brings its own page: its first
 * block spans the whole width it is given (a full-width wrapper table or
 * box). Such an email has its own margins, so the app's would only add to
 * them; a plain message (paragraphs, a signature) has none and keeps ours.
 */
export function bringsItsOwnMargins(body: HTMLElement): boolean {
  const first = [...body.children].find((child) => child.tagName !== 'STYLE');
  if (!first || !['TABLE', 'DIV', 'CENTER'].includes(first.tagName)) return false;
  const style = body.ownerDocument.defaultView?.getComputedStyle(body);
  const side = (style ? parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) : 0) || 0;
  const available = body.clientWidth - side;
  return available > 0 && first.getBoundingClientRect().width >= available - 1;
}

/** Scale a loaded email down to the screen's width if it is wider. */
export function fitToPhone(doc: Document): void {
  const body = doc.body as HTMLElement | null;
  if (!body) return;
  // Measure at full size: an earlier fit (a re-render, images arriving) must
  // not count towards the next one.
  body.style.removeProperty('zoom');
  // A phone has no width to spare: an email with its own margins gets the
  // screen edge to edge instead of sitting inside ours as well.
  body.style.removeProperty('padding-left');
  body.style.removeProperty('padding-right');
  if (bringsItsOwnMargins(body)) {
    body.style.setProperty('padding-left', '0');
    body.style.setProperty('padding-right', '0');
  }
  const pageWidth = doc.documentElement.clientWidth;
  const scale = fitScale(doc.documentElement.scrollWidth, pageWidth);
  if (scale < 1) body.style.setProperty('zoom', String(scale));
}
