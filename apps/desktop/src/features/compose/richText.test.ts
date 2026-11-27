import { MOCK_ACCOUNTS, MOCK_MESSAGES } from '../../api/mock/mockData';
import type { Account, MessageDetail } from '../../api/types';
import { htmlToPlainText, quotedOriginal, signatureBlock, signatureHtml, swapSignatureHtml, textToHtml } from './richText';
import { draftFromMessage, forwardDraft, replyDraft } from './seed';

function byId(id: string): MessageDetail {
  const m = MOCK_MESSAGES.find((x) => x.id === id);
  if (!m) throw new Error(`missing ${id}`);
  return m;
}
const [work] = MOCK_ACCOUNTS as [Account];

describe('plain text from the editor HTML', () => {
  it('keeps paragraphs, line breaks, lists, quotes, links, rules and tables readable', () => {
    const html = [
      '<p>Hi <strong>Peter</strong>,</p>',
      '<p></p>',
      '<ul><li><p>Paint</p></li><li><p>Tiles</p><ol><li><p>White</p></li><li><p>Grey</p></li></ol></li></ul>',
      '<blockquote><p>Lunch?</p><blockquote><p>Older</p></blockquote></blockquote>',
      '<p>See <a href="https://mocky.co.ke/q">the quote</a> or <a href="mailto:info@mocky.co.ke">info@mocky.co.ke</a><br>Line two</p>',
      '<hr>',
      '<table><tbody><tr><th><p>Item</p></th><th><p>Price</p></th></tr><tr><td><p>Paint</p></td><td><p>2,500</p></td></tr></tbody></table>',
      '<p>Tom &amp; Jerry &lt;3</p>',
    ].join('');
    expect(htmlToPlainText(html)).toBe(
      [
        'Hi Peter,',
        '',
        '- Paint',
        '- Tiles',
        '  1. White',
        '  2. Grey',
        '> Lunch?',
        '>> Older',
        'See the quote <https://mocky.co.ke/q> or info@mocky.co.ke',
        'Line two',
        '----------',
        'Item\tPrice',
        'Paint\t2,500',
        'Tom & Jerry <3',
      ].join('\n'),
    );
  });

  it('runs nothing and loads nothing', () => {
    expect(htmlToPlainText('<p>One</p><script>alert(1)</script><style>p{}</style><p><img src="x" alt="Logo"></p>')).toBe('One\nLogo');
  });

  it('turns plain text into paragraphs and back', () => {
    const text = 'Hi Mocky,\n\nI need a <site> & more.';
    expect(textToHtml(text)).toBe('<p>Hi Mocky,</p><p></p><p>I need a &lt;site&gt; &amp; more.</p>');
    expect(htmlToPlainText(textToHtml(text))).toBe(text);
  });
});

describe('quoted original', () => {
  it('sits in the marked block with its header line; a reply quotes it', () => {
    const html = quotedOriginal(['On Tue, Amina wrote:'], { html: '<p>Lunch?</p>', text: null }, true);
    expect(html).toBe('<div data-mocky-quote=""><p>On Tue, Amina wrote:</p><blockquote><p>Lunch?</p></blockquote></div>');
    expect(htmlToPlainText(html)).toBe('On Tue, Amina wrote:\n> Lunch?');
    // Text originals are escaped, never read as HTML.
    expect(quotedOriginal(['x'], { html: null, text: '<b>hi</b>' }, false)).toContain('&lt;b&gt;hi&lt;/b&gt;');
  });

  it('is under the cursor in a reply, with the original HTML', () => {
    const draft = replyDraft(byId('m2'), work, false, 'Africa/Nairobi');
    expect(draft.bodyHtml?.startsWith('<p></p><div data-mocky-quote="">')).toBe(true);
    expect(draft.bodyHtml).toContain('<blockquote><p>The nightly build of <b>main</b> finished without errors.</p>');
    expect(draft.bodyHtml).toMatch(/<p>On .*, Build server wrote:<\/p>/);
    // The plain-text version is still there for older backends.
    expect(draft.bodyText).toContain('> The nightly build of main finished without errors.');
  });

  it('has the forwarded header block in a forward, not quoted', () => {
    const draft = forwardDraft(byId('m1'), 'Africa/Nairobi');
    const html = draft.bodyHtml ?? '';
    expect(html).toContain('<div data-mocky-quote=""><p>---------- Forwarded message ----------<br>From: Wanjiru Kamau &lt;wanjiru@fitout.example&gt;<br>');
    expect(html).not.toContain('<blockquote>');
    expect(htmlToPlainText(html)).toContain('Subject: Revised quotation for the Westlands office fit-out');
  });

  it('continues a draft with its HTML', () => {
    expect(draftFromMessage({ ...byId('d1'), html: '<p>Friday <b>works</b></p>' }).bodyHtml).toBe('<p>Friday <b>works</b></p>');
  });
});

describe('signature in the HTML body', () => {
  const quote = '<p></p><div data-mocky-quote=""><p>On Tue, Amina wrote:</p></div>';

  it('goes above the quoted original, marked with its id, and never twice', () => {
    const body = swapSignatureHtml(quote, null, 'Don\nMocky', 'full');
    expect(body).toBe(`<p></p>${signatureHtml('Don\nMocky', 'full')}<div data-mocky-quote=""><p>On Tue, Amina wrote:</p></div>`);
    expect(body).toContain('data-mocky-signature="full"');
    expect(htmlToPlainText(body)).toContain(signatureBlock('Don\nMocky'));
    expect(swapSignatureHtml(body, null, 'Don\nMocky', 'full')).toBe(body);
  });

  it('goes last in a new message, under a line to write on', () => {
    expect(swapSignatureHtml('', null, 'Don')).toBe(`<p></p>${signatureHtml('Don')}`);
    expect(htmlToPlainText(swapSignatureHtml('', null, 'Don'))).toBe(`\n${signatureBlock('Don')}`);
  });

  it('swaps or goes, and an edited one is left alone', () => {
    const body = swapSignatureHtml(quote, null, 'Don\nMocky', 'full');
    const short = swapSignatureHtml(body, 'Don\nMocky', 'Don', 'short');
    expect(short).toContain('data-mocky-signature="short"');
    expect(htmlToPlainText(short)).toContain(signatureBlock('Don'));
    expect(htmlToPlainText(short)).not.toContain('Mocky');
    expect(swapSignatureHtml(short, 'Don', null)).toBe(quote);
    const edited = body.replace('<p>Mocky</p>', '<p>Mocky Digital</p>');
    expect(swapSignatureHtml(edited, 'Don\nMocky', 'Don', 'short')).toBe(edited);
  });
});
