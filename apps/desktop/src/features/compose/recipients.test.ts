import { addressOf, isPlausibleAddress, parseRecipients, splitRecipients } from './recipients';

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

describe('splitRecipients and addressOf', () => {
  it('keeps commas inside quotes and angle brackets', () => {
    expect(splitRecipients('"Kamau, Wanjiru" <w@x.example>, b@y.example;\nc@z.example,,')).toEqual([
      '"Kamau, Wanjiru" <w@x.example>',
      'b@y.example',
      'c@z.example',
    ]);
  });

  it('takes the address from a display-name form', () => {
    expect(addressOf('Wanjiru Kamau <w@x.example>')).toBe('w@x.example');
    expect(addressOf('w@x.example')).toBe('w@x.example');
    expect(addressOf('a <b> <c@x.example>')).toBeNull();
    expect(addressOf('"quoted"')).toBeNull();
  });
});

describe('parseRecipients', () => {
  it('accepts names, bare addresses and duplicates once', () => {
    expect(parseRecipients('Wanjiru Kamau <w@x.example>, W@X.example, b@y.example')).toEqual({
      valid: ['w@x.example', 'b@y.example'],
      invalid: [],
    });
  });

  it('reports what does not parse as typed', () => {
    expect(parseRecipients('peter@client, Bad <nope>, ok@client.example')).toEqual({
      valid: ['ok@client.example'],
      invalid: ['peter@client', 'Bad <nope>'],
    });
  });
});
