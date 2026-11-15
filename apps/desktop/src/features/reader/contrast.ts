/**
 * Readable text in HTML email.
 *
 * Senders often put light text on a background picture. The sanitizer
 * removes background pictures (they can track you), which leaves that text
 * white on white. Many also use pale grey for footers. After the email
 * renders, each piece of text is checked against what is actually behind it;
 * text below the WCAG AA contrast (4.5:1, or 3:1 for large text) is darkened
 * on light backgrounds or lightened on dark ones, keeping its hue.
 *
 * Dark mode inverts the whole rendered email afterwards, which keeps these
 * contrasts, so the check runs on the email's own (light) colours.
 */

interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** The base colours of the email document (see LIGHT_STYLE and DARK_STYLE). */
const FALLBACK_PAGE: Rgb = { r: 255, g: 255, b: 255 };

const NORMAL_TEXT = 4.5;
const LARGE_TEXT = 3;

function parse(colour: string): (Rgb & { a: number }) | null {
  const match = /rgba?\(([^)]+)\)/.exec(colour);
  if (!match?.[1]) return null;
  const parts = match[1].split(/[,\s/]+/).filter(Boolean).map(Number);
  const [r = 0, g = 0, b = 0, a = 1] = parts;
  return { r, g, b, a };
}

function channel(value: number): number {
  const v = value / 255;
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance. */
export function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG contrast ratio, 1 to 21. */
export function contrast(a: Rgb, b: Rgb): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

/** Move a colour towards black (or white) until it reads against the background. */
export function readableOn(text: Rgb, background: Rgb, target: number): Rgb {
  const towards = luminance(background) > 0.4 ? 0 : 255;
  let colour = text;
  for (let step = 1; step <= 20 && contrast(colour, background) < target; step++) {
    const mix = step / 20;
    colour = {
      r: Math.round(text.r + (towards - text.r) * mix),
      g: Math.round(text.g + (towards - text.g) * mix),
      b: Math.round(text.b + (towards - text.b) * mix),
    };
  }
  return colour;
}

function backgroundBehind(element: Element, view: Window, page: Rgb): Rgb {
  for (let current: Element | null = element; current; current = current.parentElement) {
    const colour = parse(view.getComputedStyle(current).backgroundColor);
    if (colour && colour.a > 0.5) return colour;
  }
  return page;
}

function isLarge(style: CSSStyleDeclaration): boolean {
  const size = parseFloat(style.fontSize);
  const bold = Number.parseInt(style.fontWeight, 10) >= 700;
  return size >= 24 || (bold && size >= 18.66);
}

/** Make every piece of text in a loaded email readable against its background. */
export function fixContrast(doc: Document): void {
  const view = doc.defaultView;
  const body = doc.body as HTMLElement | null;
  if (!view || !body) return;
  const page = parse(view.getComputedStyle(doc.documentElement).backgroundColor) ?? FALLBACK_PAGE;
  for (const element of body.querySelectorAll<HTMLElement>('*')) {
    let own = '';
    for (const node of element.childNodes) if (node.nodeType === 3) own += node.textContent ?? '';
    if (own.trim() === '') continue;
    const style = view.getComputedStyle(element);
    const text = parse(style.color);
    if (!text) continue;
    const background = backgroundBehind(element, view, page);
    const target = isLarge(style) ? LARGE_TEXT : NORMAL_TEXT;
    if (contrast(text, background) >= target) continue;
    const fixed = readableOn(text, background, target);
    element.style.setProperty('color', `rgb(${String(fixed.r)}, ${String(fixed.g)}, ${String(fixed.b)})`, 'important');
  }
}
