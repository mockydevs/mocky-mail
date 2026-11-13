import { isPlausibleAddress, parseRecipients } from './recipients';

describe('isPlausibleAddress', () => {
  it.each(['peter@client.example', 'a.b+tag@sub.domain.co.ke'])('accepts %s', (a) => {
    expect(isPlausibleAddress(a)).toBe(true);
  });

  it.each(['', 'peter', 'peter@', '@client.example', 'a b@x.example', 'a@x', 'a@@x.example', '<a@x.example>'])(
    'rejects %s',
    (a) => {
      expect(isPlausibleAddress(a)).toBe(false);
    },
  );
});

describe('parseRecipients', () => {
  it('splits on commas, semicolons and new lines and drops blanks', () => {
    expect(parseRecipients('a@x.example, b@y.example;\nc@z.example,,')).toEqual({
      valid: ['a@x.example', 'b@y.example', 'c@z.example'],
      invalid: [],
    });
  });

  it('separates invalid entries and removes duplicates', () => {
    expect(parseRecipients('A@x.example, a@x.example, nope')).toEqual({
      valid: ['A@x.example'],
      invalid: ['nope'],
    });
  });
});
