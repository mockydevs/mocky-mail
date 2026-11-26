import { SHORTCUTS, shortcutFor, type KeyInput } from './shortcuts';

function press(key: string, target: EventTarget | null = document.body, mods: Partial<KeyInput> = {}): ReturnType<typeof shortcutFor> {
  return shortcutFor({ key, ctrlKey: false, metaKey: false, altKey: false, target, ...mods });
}

describe('shortcutFor', () => {
  it('maps every documented key', () => {
    expect(press('j')).toBe('next');
    expect(press('#')).toBe('delete');
    expect(press('?')).toBe('help');
    expect(press('Escape')).toBe('close');
    expect(press('x')).toBeNull();
    expect(press('v')).toBe('move');
    expect(SHORTCUTS).toHaveLength(18);
  });

  it('maps Ctrl+A, Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z, but not while typing', () => {
    expect(press('a', document.body, { ctrlKey: true })).toBe('selectAll');
    expect(press('a', document.body, { metaKey: true })).toBe('selectAll');
    expect(press('z', document.body, { ctrlKey: true })).toBe('undo');
    expect(press('y', document.body, { ctrlKey: true })).toBe('redo');
    expect(press('Z', document.body, { ctrlKey: true, shiftKey: true })).toBe('redo');
    expect(press('z', document.body, { ctrlKey: true, altKey: true })).toBeNull();
    const input = document.createElement('input');
    expect(press('z', input, { ctrlKey: true })).toBeNull();
    expect(press('a', input, { ctrlKey: true })).toBeNull();
  });

  it('ignores keys while typing, Esc included, so it cannot discard what was typed', () => {
    const input = document.createElement('input');
    const textarea = document.createElement('textarea');
    const select = document.createElement('select');
    for (const target of [input, textarea, select]) {
      expect(press('r', target)).toBeNull();
      expect(press('Escape', target)).toBeNull();
    }
    expect(press('Escape', document.createElement('button'))).toBe('close');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    expect(press('j', checkbox)).toBe('next');
  });

  it('leaves modifier combinations and Enter on buttons alone', () => {
    expect(press('r', document.body, { ctrlKey: true })).toBeNull();
    expect(press('c', document.body, { metaKey: true })).toBeNull();
    expect(press('Enter', document.createElement('button'))).toBeNull();
    expect(press('Enter', document.body)).toBe('open');
  });
});
