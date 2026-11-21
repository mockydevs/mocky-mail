import { afterEach, vi } from 'vitest';

import { fitScale, fitToPhone, MIN_SCALE, PHONE_STYLE } from './phoneLayout';

/** jsdom has no layout: give the page the widths a 390px phone would. */
function page(contentWidth: number, pageWidth = 390) {
  vi.spyOn(document.documentElement, 'clientWidth', 'get').mockReturnValue(pageWidth);
  vi.spyOn(document.documentElement, 'scrollWidth', 'get').mockReturnValue(contentWidth);
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = '';
  document.body.removeAttribute('style');
});

describe('fitScale', () => {
  it('leaves an email that fits at full size', () => {
    expect(fitScale(390, 390)).toBe(1);
    expect(fitScale(200, 390)).toBe(1);
    // A pixel of rounding is not worth shrinking for.
    expect(fitScale(391, 390)).toBe(1);
  });

  it('shrinks a desktop-only email to the screen', () => {
    expect(fitScale(600, 390)).toBeCloseTo(0.65);
  });

  it('never shrinks past readable; a very wide email scrolls instead', () => {
    expect(fitScale(2000, 390)).toBe(MIN_SCALE);
    expect(fitScale(600, 0)).toBe(1);
  });
});

describe('fitToPhone', () => {
  it('keeps the email exactly as designed: nothing is moved, stacked or stripped', () => {
    page(390);
    const html =
      '<table width="100%" cellpadding="0"><tr><td style="padding:0 24px">Service</td><td align="right">Renewed until</td></tr></table>';
    document.body.innerHTML = html;
    const before = document.body.innerHTML;
    fitToPhone(document);
    expect(document.body.innerHTML).toBe(before);
    expect(document.body.style.getPropertyValue('zoom')).toBe('');
  });

  it('scales an email wider than the screen down to fit, whole', () => {
    page(600);
    document.body.innerHTML = '<table width="600"><tr><td>Wide</td><td>Receipt</td></tr></table>';
    fitToPhone(document);
    expect(Number(document.body.style.getPropertyValue('zoom'))).toBeCloseTo(0.65);
  });

  it('measures afresh each time, so fitting twice does not shrink twice', () => {
    page(600);
    fitToPhone(document);
    fitToPhone(document);
    expect(Number(document.body.style.getPropertyValue('zoom'))).toBeCloseTo(0.65);
  });

  it('gives a designed email the full width, since it brings its own margins', () => {
    page(390);
    vi.spyOn(document.body, 'clientWidth', 'get').mockReturnValue(390);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 390, height: 100, top: 0, left: 0, right: 390, bottom: 100, x: 0, y: 0, toJSON: () => ({}),
    });
    document.body.innerHTML = '<style>p{margin:0}</style><table width="100%"><tr><td>Receipt</td></tr></table>';
    fitToPhone(document);
    expect(document.body.style.getPropertyValue('padding-left')).toBe('0px');
    expect(document.body.style.getPropertyValue('padding-right')).toBe('0px');
  });

  it('keeps side padding for a plain message with no margins of its own', () => {
    page(390);
    vi.spyOn(document.body, 'clientWidth', 'get').mockReturnValue(390);
    document.body.innerHTML = '<p>Thanks, see you Monday.</p>';
    fitToPhone(document);
    expect(document.body.style.getPropertyValue('padding-left')).toBe('');
  });

  it('only adds safe base rules: images and code stay inside the screen', () => {
    expect(PHONE_STYLE).toContain('img{max-width:100%');
    expect(PHONE_STYLE).toContain('pre,code{white-space:pre-wrap');
    // Tables and cells keep the widths the email gave them.
    expect(PHONE_STYLE).not.toMatch(/table|td/);
  });
});
