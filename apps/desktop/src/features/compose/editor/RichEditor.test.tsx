import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { vi } from 'vitest';

import { editorOf, messageHtml, selectText } from '../../../test/editor';
import type { EditorHandle } from './loader';
import { RichEditor } from './RichEditor';

function renderEditor(value = '<p></p>') {
  const user = userEvent.setup();
  const onChange = vi.fn<(html: string, text: string) => void>();
  const onNormalize = vi.fn<(from: string, to: string, text: string) => void>();
  const handle = createRef<EditorHandle>();
  render(
    <RichEditor label="Message" value={value} onChange={onChange} onNormalize={onNormalize} focusOnOpen={false} handleRef={handle} phone={false} />,
  );
  const box = screen.getByRole('textbox', { name: 'Message' });
  const last = () => onChange.mock.calls.at(-1) ?? ['', ''];
  const tool = (name: string) => screen.getByRole('button', { name });
  return { user, box, onChange, onNormalize, handle, last, tool };
}

async function typed(text: string) {
  const ctx = renderEditor();
  await ctx.user.click(ctx.box);
  await ctx.user.keyboard(text);
  return ctx;
}

describe('rich-text editor', () => {
  it('reports HTML and a plain-text version on every change', async () => {
    const { last } = await typed('Hello world');
    expect(last()).toEqual(['<p>Hello world</p>', 'Hello world']);
  });

  it('formats with the toolbar and the keyboard', async () => {
    const { user, box, tool, last } = await typed('Hello world');
    selectText(box, 'Hello');
    await user.click(tool('Underline'));
    expect(last()[0]).toBe('<p><u>Hello</u> world</p>');
    await user.keyboard('{Control>}b{/Control}');
    expect(last()[0]).toBe('<p><strong><u>Hello</u></strong> world</p>');
    await user.click(tool('Strikethrough'));
    expect(last()[0]).toContain('<s><u>Hello</u></s>');
    expect(tool('Strikethrough')).toHaveAttribute('aria-pressed', 'true');

    selectText(box, 'world');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Font' }), 'Georgia');
    expect(last()[0]).toContain('<span style="font-family: Georgia, serif;">world</span>');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Size' }), 'Large');
    expect(last()[0]).toContain('font-size: 18px');
    await user.click(screen.getByRole('button', { name: 'Text colour' }));
    await user.click(screen.getByRole('menuitemradio', { name: /Red/ }));
    expect(last()[0]).toContain('color: rgb(192, 57, 43)');
    await user.click(screen.getByRole('button', { name: 'Highlight' }));
    await user.click(screen.getByRole('menuitemradio', { name: /Yellow/ }));
    expect(last()[0]).toMatch(/<mark data-color="#fff59d"[^>]*>/);
    await user.click(screen.getByRole('button', { name: 'Alignment' }));
    await user.click(screen.getByRole('menuitemradio', { name: 'Centre' }));
    expect(last()[0]).toMatch(/^<p style="text-align: center;?">/);

    selectText(box, 'Hello');
    await user.click(tool('Clear formatting'));
    expect(last()[0]).not.toContain('<u>');
    expect(last()[0]).not.toContain('<strong>');
  });

  it('makes lists, nested at most eight deep with Tab, and indents paragraphs', async () => {
    const { user, tool, last } = await typed('one');
    await user.click(tool('Bulleted list'));
    expect(last()[0]).toBe('<ul><li><p>one</p></li></ul>');
    for (let i = 0; i < 10; i++) {
      await user.keyboard(`{Enter}{Tab}L${String(i + 2)}`);
    }
    const depth = (html: string) => (html.split('</ul>')[0] ?? '').split('<ul>').length - 1;
    expect(depth(last()[0])).toBe(8);
    expect(last()[1]).toContain('              - L8');
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(last()[1]).toContain('            - L9');
  });

  it('makes numbered lists', async () => {
    const { user, tool, last } = await typed('first');
    await user.click(tool('Numbered list'));
    expect(last()[0]).toBe('<ol><li><p>first</p></li></ol>');
    expect(last()[1]).toBe('1. first');
  });

  it('indents a paragraph and takes it back', async () => {
    const { user, tool, last } = await typed('Indented');
    await user.click(tool('Increase indent'));
    expect(last()[0]).toBe('<p style="margin-left: 2em;">Indented</p>');
    await user.click(tool('Decrease indent'));
    expect(last()[0]).toBe('<p>Indented</p>');
  });

  it('inserts web and email links only, from a dialog with Ctrl+K', async () => {
    const { user, box, last } = await typed('See the quote');
    selectText(box, 'quote');
    await user.keyboard('{Control>}k{/Control}');
    const dialog = within(screen.getByRole('dialog', { name: 'Insert link' }));
    expect(dialog.getByLabelText('Text to show')).toHaveValue('quote');
    await user.click(dialog.getByLabelText('Address'));
    await user.paste('javascript:alert(1)');
    await user.click(dialog.getByRole('button', { name: 'Insert link' }));
    expect(dialog.getByRole('alert')).toHaveTextContent('Use a web address');
    await user.clear(dialog.getByLabelText('Address'));
    await user.paste('mocky.co.ke/q');
    await user.keyboard('{Enter}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(last()[0]).toBe('<p>See the <a target="_blank" rel="noopener noreferrer nofollow" href="https://mocky.co.ke/q">quote</a></p>');
    expect(last()[1]).toBe('See the quote <https://mocky.co.ke/q>');

    selectText(box, 'quote');
    await user.click(screen.getByRole('button', { name: 'Link' }));
    const edit = within(screen.getByRole('dialog', { name: 'Edit link' }));
    expect(edit.getByLabelText('Address')).toHaveValue('https://mocky.co.ke/q');
    await user.click(edit.getByRole('button', { name: 'Remove link' }));
    expect(last()[0]).toBe('<p>See the quote</p>');

    await user.click(box);
    await user.keyboard('{Control>}k{/Control}');
    const mail = within(screen.getByRole('dialog', { name: 'Insert link' }));
    await user.click(mail.getByLabelText('Address'));
    await user.paste('info@mocky.co.ke');
    await user.keyboard('{Enter}');
    expect(last()[0]).toContain('href="mailto:info@mocky.co.ke">info@mocky.co.ke</a>');
  });

  it('quotes, draws a line, and makes tables with rows and columns to add and remove', async () => {
    const { user, tool, last } = await typed('Quoted');
    await user.click(tool('Quote'));
    expect(last()[0]).toBe('<blockquote><p>Quoted</p></blockquote>');
    await user.click(tool('Horizontal line'));
    expect(last()[0]).toContain('<hr>');

    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Insert table (3 × 3)' }));
    const rows = () => (last()[0].match(/<tr>/g) ?? []).length;
    const cells = () => (last()[0].split('</tr>')[0]?.match(/<td/g) ?? []).length;
    expect(rows()).toBe(3);
    expect(cells()).toBe(3);
    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Add row below' }));
    expect(rows()).toBe(4);
    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Add column right' }));
    expect(cells()).toBe(4);
    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Delete column' }));
    expect(cells()).toBe(3);
    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Delete row' }));
    expect(rows()).toBe(3);
    await user.click(screen.getByRole('button', { name: 'Table' }));
    await user.click(screen.getByRole('menuitem', { name: 'Delete table' }));
    expect(last()[0]).not.toContain('<table');
  });

  it('keeps pasted formatting it knows and drops anything unsafe', () => {
    const { box, last } = renderEditor();
    const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
    const pasted = [
      '<p onclick="steal()">Hi <b>there</b> <a href="javascript:steal()">click</a> <a href="https://ok.example">ok</a></p>',
      '<script>steal()</script><iframe src="https://evil.example"></iframe>',
      `<p><img src="https://tracker.example/pixel.gif"><img src="${PNG}"></p>`,
      '<form><input value="card"></form>',
    ].join('');
    act(() => {
      editorOf(box).commands.focus();
    });
    fireEvent.paste(box, {
      clipboardData: {
        types: ['text/html', 'text/plain'],
        getData: (type: string) => (type === 'text/html' ? pasted : 'Hi there click ok'),
      },
    });
    const html = last()[0];
    expect(html).toContain('<strong>there</strong>');
    expect(html).toContain('href="https://ok.example"');
    for (const bad of ['onclick', 'javascript:', '<script', 'steal', 'iframe', 'evil.example', 'tracker.example', '<input', '<form']) {
      expect(html).not.toContain(bad);
    }
    // A picture inside the message survives; the backend sends it as a part.
    expect(html).toContain(`<img src="${PNG}">`);
  });

  it('keeps the quote and signature marks the backend looks for', () => {
    const value =
      '<p>Thanks</p><div data-mocky-signature="s1"><p>--&nbsp;</p><p>Don</p></div><div data-mocky-quote=""><p>On Tue, Amina wrote:</p><blockquote><p>Lunch?</p></blockquote></div>';
    const { box, onNormalize } = renderEditor(value);
    expect(messageHtml(box)).toBe(value.replace('--&nbsp;', '--&nbsp;'));
    expect(onNormalize).not.toHaveBeenCalled();
  });

  it('reads the quote mark on a blockquote too (the first 0.7.0 form)', () => {
    const { box } = renderEditor('<blockquote data-mocky-quote><p>Earlier</p></blockquote>');
    expect(messageHtml(box)).toBe('<div data-mocky-quote=""><p>Earlier</p></div>');
  });

  it('finds and replaces, with a count, next and previous, and match case', async () => {
    const { user, box, last } = await typed('Cat cat CAT dog');
    await user.keyboard('{Control>}f{/Control}');
    const bar = within(screen.getByRole('search', { name: 'Find and replace' }));
    const find = bar.getByRole('textbox', { name: 'Find' });
    expect(find).toHaveFocus();
    await user.type(find, 'cat');
    expect(bar.getByRole('status')).toHaveTextContent('1 of 3');
    expect(box.querySelectorAll('.rte-find')).toHaveLength(3);
    await user.click(bar.getByRole('button', { name: 'Next match' }));
    expect(bar.getByRole('status')).toHaveTextContent('2 of 3');
    await user.click(bar.getByRole('button', { name: 'Previous match' }));
    await user.click(bar.getByRole('button', { name: 'Previous match' }));
    expect(bar.getByRole('status')).toHaveTextContent('3 of 3');
    await user.click(bar.getByRole('checkbox', { name: 'Match case' }));
    expect(bar.getByRole('status')).toHaveTextContent('1 of 1');
    await user.click(bar.getByRole('checkbox', { name: 'Match case' }));

    await user.keyboard('{Control>}h{/Control}');
    const replace = bar.getByRole('textbox', { name: 'Replace with' });
    await user.type(replace, 'bird');
    // The current match (the third, where the search left off) goes first.
    expect(bar.getByRole('status')).toHaveTextContent('3 of 3');
    await user.click(bar.getByRole('button', { name: 'Replace' }));
    expect(last()[0]).toBe('<p>Cat cat bird dog</p>');
    expect(bar.getByRole('status')).toHaveTextContent('1 of 2');
    await user.click(bar.getByRole('button', { name: 'Replace all' }));
    expect(last()[0]).toBe('<p>bird bird bird dog</p>');
    expect(bar.getByRole('status')).toHaveTextContent('Replaced 2');
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('search')).not.toBeInTheDocument();
    expect(box.querySelectorAll('.rte-find')).toHaveLength(0);
  });

  it('copies formatting with the painter: once on a click, until Esc on a double-click', async () => {
    const { user, box, tool, last } = await typed('Bold one two three');
    selectText(box, 'Bold');
    await user.click(tool('Bold'));
    await user.click(tool('Italic'));
    selectText(box, 'Bold');
    await user.click(tool('Format painter'));
    expect(tool('Format painter')).toHaveAttribute('aria-pressed', 'true');
    selectText(box, 'one');
    fireEvent.mouseUp(box);
    expect(last()[0]).toBe('<p><strong><em>Bold</em></strong> <strong><em>one</em></strong> two three</p>');
    expect(tool('Format painter')).toHaveAttribute('aria-pressed', 'false');
    // Not again without asking.
    selectText(box, 'two');
    fireEvent.mouseUp(box);
    expect(last()[0]).toContain(' two three');

    selectText(box, 'Bold');
    await user.dblClick(tool('Format painter'));
    expect(screen.getByRole('button', { name: 'Format painter (on until Esc)' })).toHaveAttribute('aria-pressed', 'true');
    selectText(box, 'two');
    fireEvent.mouseUp(box);
    selectText(box, 'three');
    fireEvent.mouseUp(box);
    expect(last()[0]).toBe('<p><strong><em>Bold</em></strong> <strong><em>one</em></strong> <strong><em>two</em></strong> <strong><em>three</em></strong></p>');
    act(() => {
      editorOf(box).commands.focus();
    });
    await user.keyboard('{Escape}');
    expect(tool('Format painter')).toHaveAttribute('aria-pressed', 'false');
  });

  it('types a symbol at the cursor for Insert', async () => {
    const { handle, last } = await typed('Price 5');
    act(() => {
      handle.current?.insertText('€');
    });
    expect(last()[0]).toBe('<p>Price 5€</p>');
  });

  it('shows a value from outside (a signature swap) without reporting it as typing', () => {
    const onChange = vi.fn();
    const onNormalize = vi.fn();
    const props = { label: 'Message', onChange, onNormalize, focusOnOpen: false, handleRef: createRef<EditorHandle>(), phone: false };
    const { rerender } = render(<RichEditor {...props} value="<p>One</p>" />);
    rerender(<RichEditor {...props} value="<p>One</p><div data-mocky-signature=&quot;a&quot;><p>Two</p></div>" />);
    expect(messageHtml(screen.getByRole('textbox', { name: 'Message' }))).toContain('<p>Two</p>');
    expect(onChange).not.toHaveBeenCalled();
  });
});
