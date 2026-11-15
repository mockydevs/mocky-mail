import { afterEach, vi } from 'vitest';

import { fitToPhone, PHONE_STYLE } from './phoneLayout';

/** jsdom has no layout: give elements the widths a 390px phone would. */
function widths(map: Record<string, number>) {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const width = map[this.id] ?? 390;
    return { width, height: 20, top: 0, left: 0, right: width, bottom: 20, x: 0, y: 0, toJSON: () => ({}) };
  });
  vi.spyOn(document.documentElement, 'clientWidth', 'get').mockReturnValue(390);
}

function email(html: string): Document {
  document.body.innerHTML = html;
  return document;
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = '';
  document.documentElement.removeAttribute('style');
  document.body.removeAttribute('style');
});

describe('fitToPhone', () => {
  it('drops the side padding, margins and borders that box the content in', () => {
    widths({});
    const doc = email(
      '<table id="outer" width="600" style="background-color:#ffffff"><tr><td id="gutter" style="padding:0 24px">' +
        '<div id="card" style="border:1px solid #ddd;border-radius:12px;margin:0 12px;padding:20px">Text</div>' +
        '</td></tr></table>',
    );
    fitToPhone(doc);
    const card = doc.getElementById('card');
    expect(doc.getElementById('gutter')?.style.paddingLeft).toBe('0px');
    expect(card?.style.paddingRight).toBe('0px');
    expect(card?.style.marginLeft).toBe('0px');
    expect(card?.style.borderLeftWidth).toBe('0px');
    // A 600px table is held to the screen width.
    expect(doc.getElementById('outer')?.style.width).toBe('100%');
    // The page takes the email's background, so no band frames it.
    expect(doc.body.style.backgroundColor).toBe('rgb(255, 255, 255)');
  });

  it('keeps padding on buttons', () => {
    widths({ button: 160 });
    const doc = email('<table><tr><td id="button" style="background-color:#000;padding:12px 20px"><a href="https://x.example">Go</a></td></tr></table>');
    fitToPhone(doc);
    expect(doc.getElementById('button')?.style.paddingLeft).toBe('20px');
  });

  it('stretches a coloured footer card across the screen instead of shrinking it', () => {
    widths({ footer: 130 });
    const doc = email(
      '<table id="wrap" style="max-width:600px"><tr><td id="footer" style="background-color:#ededed;border-radius:12px;padding:24px">' +
        '<img src="cid:logo" alt="Logo"><p>1455 3rd Street</p><p>Unsubscribe · Privacy · Terms</p></td></tr></table>',
    );
    fitToPhone(doc);
    const footer = doc.getElementById('footer');
    expect(footer?.style.width).toBe('100%');
    expect(doc.getElementById('wrap')?.style.width).toBe('100%');
    // Text keeps a little room from the card's edge.
    expect(footer?.style.paddingLeft).toBe('16px');
  });

  it('stacks side-by-side columns too narrow to read', () => {
    widths({ picture: 150, words: 120, img: 140 });
    const doc = email(
      '<table><tr><td id="picture"><img id="img" src="cid:a" alt=""></td>' +
        '<td id="words">Connect your finances and let it review where your money goes.</td></tr></table>',
    );
    fitToPhone(doc);
    expect(doc.getElementById('words')?.style.display).toBe('block');
    expect(doc.getElementById('picture')?.style.width).toBe('100%');
  });

  it('leaves an icon beside its label alone', () => {
    widths({ icon: 24, label: 300 });
    const doc = email(
      '<table><tr><td id="icon"><img src="cid:i" alt=""></td><td id="label">Adee Wyclife and 30 others like this post.</td></tr></table>',
    );
    fitToPhone(doc);
    expect(doc.getElementById('label')?.style.display).toBe('');
  });

  it('stacks a cramped row by turning its table into blocks, so cells leave the grid', () => {
    widths({ a: 51, b: 51, c: 51, d: 51 });
    const doc = email(
      '<table id="grid"><tbody id="rows"><tr id="row"><td id="a">Rating date</td><td id="b">Tuesday, November 10, 2026</td>' +
        '<td id="c">Storefront</td><td id="d">Google Play</td></tr></tbody></table>',
    );
    fitToPhone(doc);
    for (const id of ['grid', 'rows', 'row', 'a', 'b', 'c', 'd']) {
      expect(doc.getElementById(id)?.style.display).toBe('block');
    }
  });

  it('stacks narrow columns that are not table cells', () => {
    widths({ wrap: 341, picture: 106, words: 83, wbody: 83, wrow: 83, wcell: 83, text: 83 });
    const doc = email(
      '<div id="wrap"><table id="picture"><tr><td><img src="cid:a" alt=""></td></tr></table>' +
        '<table id="words"><tbody id="wbody"><tr id="wrow"><td id="wcell">' +
        '<p id="text">Get a fast and focused email experience with the desktop app.</p></td></tr></tbody></table></div>',
    );
    fitToPhone(doc);
    expect(doc.getElementById('words')?.style.display).toBe('block');
    expect(doc.getElementById('picture')?.style.width).toBe('100%');
  });

  it('holds wide pictures and one-line text to the screen', () => {
    widths({ banner: 600, row: 456 });
    const doc = email(
      '<table><tr id="row"><td id="date" style="white-space:nowrap">10 Nov 2026</td><td><img id="banner" width="600" src="cid:b" alt=""></td></tr></table>',
    );
    fitToPhone(doc);
    expect(doc.getElementById('banner')?.style.width).toBe('100%');
    expect(doc.getElementById('date')?.style.whiteSpace).toBe('normal');
  });

  it('raises text too small to read', () => {
    widths({});
    const doc = email('<p id="legal" style="font-size:9px">Unsubscribe from these emails at any time.</p>');
    fitToPhone(doc);
    expect(doc.getElementById('legal')?.style.fontSize).toBe('11px');
  });

  it('wraps preformatted text and rules on a phone', () => {
    expect(PHONE_STYLE).toContain('pre,code{white-space:pre-wrap!important');
    expect(PHONE_STYLE).toContain('hr{max-width:100%!important');
  });
});
