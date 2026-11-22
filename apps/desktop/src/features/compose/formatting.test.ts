import { link, prefixLines, wrap } from './formatting';

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
  it('formats with the toolbar and Ctrl+B, and previews what recipients see', async () => {
    const { renderApp, rows } = await import('../../test/renderApp');
    const { screen, within, waitFor } = await import('@testing-library/react');
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'New message' }));
    const form = within(screen.getByRole('form', { name: 'New message' }));
    const body = form.getByLabelText<HTMLTextAreaElement>('Message');
    await user.click(body);
    await user.keyboard('{Control>}a{/Control}');
    await user.keyboard('Please confirm');
    body.setSelectionRange(7, 14);
    await user.click(form.getByRole('button', { name: 'Bold' }));
    expect(body.value.startsWith('Please **confirm**')).toBe(true);
    // The toolbar puts the selection back on the next frame; wait for that
    // before choosing a new one, or it lands on top of ours.
    await waitFor(() => {
      expect(body.selectionStart).toBe(9);
    });

    await user.click(body);
    body.setSelectionRange(0, 6);
    await user.keyboard('{Control>}i{/Control}');
    expect(body.value.startsWith('*Please* **confirm**')).toBe(true);

    await user.click(form.getByRole('button', { name: 'Preview' }));
    const frame = await form.findByTitle('Preview of your message');
    await waitFor(() => {
      expect(frame.getAttribute('srcdoc')).toContain('<strong>confirm</strong>');
    });
    expect(body).not.toBeVisible();
    await user.click(form.getByRole('button', { name: 'Edit' }));
    expect(body).toBeVisible();
  });
});
