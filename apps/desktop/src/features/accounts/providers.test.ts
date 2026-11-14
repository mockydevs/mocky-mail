import { accountErrorCopy, appPasswordProvider, domainOf, guessServers, isValidServer } from './providers';

describe('providers', () => {
  it('finds domains and app-password providers', () => {
    expect(domainOf('Don@Gmail.com ')).toBe('gmail.com');
    expect(domainOf('nope')).toBe('');
    expect(appPasswordProvider('a@gmail.com')).toBe('Gmail');
    expect(appPasswordProvider('a@yahoo.co.uk')).toBe('Yahoo');
    expect(appPasswordProvider('a@icloud.com')).toBe('iCloud');
    expect(appPasswordProvider('a@mocky.co.ke')).toBeNull();
  });

  it('guesses servers and validates them', () => {
    const guess = guessServers('a@mocky.co.ke');
    expect(guess.imap).toEqual({ host: 'imap.mocky.co.ke', port: 993, security: 'tls' });
    expect(isValidServer(guess.smtp)).toBe(true);
    expect(isValidServer({ host: 'bad host', port: 993, security: 'tls' })).toBe(false);
    expect(isValidServer({ host: 'a.b', port: 70000, security: 'tls' })).toBe(false);
  });

  it('explains errors by kind with provider-specific help', () => {
    const auth = accountErrorCopy({ kind: 'auth', message: 'Rejected' }, 'a@gmail.com');
    expect(auth.title).toBe('Sign-in was not accepted');
    expect(auth.hint).toContain('Gmail needs an app password');
    expect(accountErrorCopy({ kind: 'tls', message: 'x' }, 'a@b.example').hint).toContain('STARTTLS');
    expect(accountErrorCopy({ kind: 'internal', message: 'x' }, 'a@b.example').hint).toBeNull();
  });
});
