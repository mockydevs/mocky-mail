import { appError, errorTitle, isAppError, MailError, toAppError } from './errors';
import { APP_ERROR_KINDS } from './types';

describe('AppError helpers', () => {
  it('recognises only well-formed AppErrors', () => {
    expect(isAppError({ kind: 'auth', message: 'Wrong password' })).toBe(true);
    expect(isAppError({ kind: 'nope', message: 'x' })).toBe(false);
    expect(isAppError({ kind: 'auth' })).toBe(false);
    expect(isAppError('auth')).toBe(false);
    expect(isAppError(null)).toBe(false);
  });

  it('keeps AppErrors and hides anything else behind a generic message', () => {
    const tls = toAppError({ kind: 'tls', message: 'bad cert' });
    expect(tls).toBeInstanceOf(MailError);
    expect(tls).toMatchObject({ kind: 'tls', message: 'bad cert' });
    const existing = appError('auth', 'no');
    expect(toAppError(existing)).toBe(existing);
    expect(isAppError(existing)).toBe(true);
    const fromString = toAppError('panicked at src/main.rs: token=abc123');
    expect(fromString.kind).toBe('internal');
    expect(fromString.message).not.toContain('abc123');
    expect(toAppError(new Error('boom')).kind).toBe('internal');
  });

  it('has a title for every kind', () => {
    for (const kind of APP_ERROR_KINDS) expect(errorTitle(kind)).not.toBe('');
  });
});
