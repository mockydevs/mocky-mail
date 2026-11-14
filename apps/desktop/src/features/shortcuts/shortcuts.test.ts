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
    expect(SHORTCUTS).toHaveLength(14);
  });

  it('ignores keys while typing, except Esc', () => {
    const input = document.createElement('input');
    const textarea = document.createElement('textarea');
    for (const target of [input, textarea]) {
      expect(press('r', target)).toBeNull();
      expect(press('Escape', target)).toBe('close');
    }
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
