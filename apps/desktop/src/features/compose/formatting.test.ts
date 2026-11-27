import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { messageHtml, selectText } from '../../test/editor';
import { renderApp, rows } from '../../test/renderApp';
import { link, prefixLines, wrap } from './formatting';
import { preloadEditor } from './editor/loader';

// Compose's editor is a chunk of its own: have it here before the first test.
beforeAll(async () => {
  await preloadEditor();
});

describe('compose formatting', () => {
  it('wraps the selection, or a placeholder, and unwraps on a second press', () => {
    expect(wrap('Please confirm today', 7, 14, '**', 'bold text')).toEqual({
      value: 'Please **confirm** today',
      start: 9,
      end: 16,
    });
    const empty = wrap('Hi ', 3, 3, '*', 'italic text');
    expect(empty.value).toBe('Hi *italic text*');
    expect(empty.value.slice(empty.start, empty.end)).toBe('italic text');
    expect(wrap('Please **confirm** today', 9, 16, '**', 'x').value).toBe('Please confirm today');
  });

  it('makes links from text or from a pasted address', () => {
    const fromText = link('See the quote', 8, 13);
    expect(fromText.value).toBe('See the [quote](https://)');
    expect(fromText.value.slice(fromText.start, fromText.end)).toBe('https://');
    const fromUrl = link('https://mocky.co.ke', 0, 19);
    expect(fromUrl.value).toBe('[link text](https://mocky.co.ke)');
    expect(fromUrl.value.slice(fromUrl.start, fromUrl.end)).toBe('link text');
  });

  it('makes and unmakes lists and quotes line by line', () => {
    const text = 'Need:\nPaint\nTiles\nThanks';
    const bullets = prefixLines(text, 6, 17, '- ');
    expect(bullets.value).toBe('Need:\n- Paint\n- Tiles\nThanks');
    expect(prefixLines(bullets.value, bullets.start, bullets.end, '- ').value).toBe(text);
    expect(prefixLines(text, 6, 17, '', true).value).toBe('Need:\n1. Paint\n2. Tiles\nThanks');
    expect(prefixLines('Lunch?', 0, 0, '> ').value).toBe('> Lunch?');
  });
});

describe('compose formatting in the app', () => {
  it('formats with the toolbar and Ctrl+I, and sends what recipients see as HTML', async () => {
    const { user, api } = renderApp();
    await rows();
    const send = vi.spyOn(api, 'sendMessage');
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = within(screen.getByRole('form', { name: 'New message' }));
    await user.type(form.getByLabelText('To'), 'peter@client.example');
    const body = await form.findByLabelText('Message');
    await user.click(body);
    await user.keyboard('Please confirm');
    selectText(body, 'confirm');
    await user.click(form.getByRole('button', { name: 'Bold' }));
    expect(form.getByRole('button', { name: 'Bold' })).toHaveAttribute('aria-pressed', 'true');
    expect(messageHtml(body)).toContain('<strong>confirm</strong>');
    selectText(body, 'Please');
    await user.keyboard('{Control>}i{/Control}');
    expect(messageHtml(body)).toContain('<em>Please</em>');

    await user.click(form.getByRole('button', { name: 'Send' }));
    await vi.waitFor(() => {
      expect(send).toHaveBeenCalledWith(
        expect.objectContaining({ bodyHtml: '<p><em>Please</em> <strong>confirm</strong></p>', bodyText: 'Please confirm' }),
      );
    });
  });
});
