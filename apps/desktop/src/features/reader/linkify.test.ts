import { linkify } from './linkify';

describe('linkify', () => {
  it('finds web and mail links and trims trailing punctuation', () => {
    expect(linkify('See https://a.example/q?x=1. Or mailto:b@c.example!')).toEqual([
      { kind: 'text', text: 'See ' },
      { kind: 'link', text: 'https://a.example/q?x=1', url: 'https://a.example/q?x=1' },
      { kind: 'text', text: '. Or ' },
      { kind: 'link', text: 'mailto:b@c.example', url: 'mailto:b@c.example' },
      { kind: 'text', text: '!' },
    ]);
  });

  it('ignores other schemes', () => {
    expect(linkify('javascript:alert(1) file:///etc/passwd')).toEqual([
      { kind: 'text', text: 'javascript:alert(1) file:///etc/passwd' },
    ]);
  });
});
