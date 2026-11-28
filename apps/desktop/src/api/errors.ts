import { APP_ERROR_KINDS, type AppError, type AppErrorKind } from './types';

/**
 * An AppError as a real Error, so stack traces and `instanceof Error` work.
 * It has the same `kind` and `message` fields as the contract's AppError.
 */
export class MailError extends Error implements AppError {
  readonly kind: AppErrorKind;
  /** Microsoft 365: the page an administrator opens to approve Mocky Mail (see AppError). */
  readonly adminConsentUrl?: string;

  constructor(kind: AppErrorKind, message: string, adminConsentUrl?: string) {
    super(message);
    this.name = 'MailError';
    this.kind = kind;
    if (adminConsentUrl !== undefined) this.adminConsentUrl = adminConsentUrl;
  }
}

/** Only an https page is ever offered as the administrator's approval page. */
function consentUrlOf(value: unknown): string | undefined {
  return typeof value === 'string' && /^https:\/\/\S+$/i.test(value) ? value : undefined;
}

export function isAppError(value: unknown): value is AppError {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v['message'] === 'string' &&
    typeof v['kind'] === 'string' &&
    (APP_ERROR_KINDS as readonly string[]).includes(v['kind'])
  );
}

/**
 * Normalise any rejection into an AppError. The backend always rejects with
 * one, but a crashed command or a JS bug can produce a string or Error; those
 * become `internal` with a generic message so nothing unvetted is shown.
 */
export function toAppError(error: unknown): MailError {
  if (error instanceof MailError) return error;
  if (isAppError(error)) return new MailError(error.kind, error.message, consentUrlOf(error.adminConsentUrl));
  return new MailError('internal', 'Something went wrong. Please try again.');
}

export function appError(kind: AppErrorKind, message: string, adminConsentUrl?: string): MailError {
  return new MailError(kind, message, adminConsentUrl);
}

/** A short headline for each kind; the backend message gives the detail. */
export function errorTitle(kind: AppErrorKind): string {
  switch (kind) {
    case 'auth':
      return 'Sign-in was not accepted';
    case 'connect':
      return 'Could not reach the mail server';
    case 'tls':
      return 'Secure connection failed';
    case 'config':
      return 'Not set up yet';
    case 'not_found':
      return 'Not found';
    case 'invalid':
      return 'Please check this';
    case 'too_large':
      return 'Too large to send';
    case 'storage':
      return 'Could not save on this device';
    case 'keyring':
      return 'Could not use the system password store';
    case 'internal':
      return 'Something went wrong';
  }
}
