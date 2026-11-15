import { afterEach, vi } from 'vitest';

import { fitToPhone } from './phoneLayout';

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
});
