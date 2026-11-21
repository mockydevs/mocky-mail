import type { AppError, ServerConfig, ServerSuggestion } from '../../api/types';
import { errorTitle } from '../../api/errors';

const APP_PASSWORD_PROVIDERS: readonly [RegExp, string][] = [
  [/^(gmail|googlemail)\.com$/, 'Gmail'],
  [/^(yahoo|ymail|rocketmail)\.[a-z.]+$/, 'Yahoo'],
  [/^(icloud|me|mac)\.com$/, 'iCloud'],
  [/^(aol|aim)\.com$/, 'AOL'],
];

export function domainOf(email: string): string {
  const at = email.lastIndexOf('@');
  return at < 0 ? '' : email.slice(at + 1).trim().toLowerCase();
}

/** Providers that refuse your normal password for IMAP and want an app password. */
export function appPasswordProvider(email: string): string | null {
  const domain = domainOf(email);
  return APP_PASSWORD_PROVIDERS.find(([pattern]) => pattern.test(domain))?.[1] ?? null;
}

/** Reasonable starting values when the backend has no suggestion. */
export function guessServers(email: string): ServerSuggestion {
  const domain = domainOf(email) || 'example.com';
  return {
    imap: { host: `imap.${domain}`, port: 993, security: 'tls' },
    smtp: { host: `smtp.${domain}`, port: 465, security: 'tls' },
    auth: 'password',
  };
}

export function isValidServer(server: ServerConfig): boolean {
  return /^[a-z0-9.-]+$/i.test(server.host) && server.host.includes('.') && Number.isInteger(server.port) && server.port > 0 && server.port < 65536;
}

export interface ErrorCopy {
  title: string;
  message: string;
  hint: string | null;
}

/** What to tell the user when adding or updating an account fails. */
export function accountErrorCopy(error: AppError, email: string, via: 'password' | 'oauth' = 'password'): ErrorCopy {
  const provider = appPasswordProvider(email);
  let hint: string | null = null;
  switch (error.kind) {
    case 'auth':
      hint = via === 'oauth'
        ? 'Try again, and on Google’s or Microsoft’s screen choose the account and allow access to your mail. Or add the account with an app password below.'
        : provider
        ? `${provider} needs an app password for mail apps, not your normal password. Create one in your ${provider} account security settings and paste it here.`
        : 'Check the password. Some providers need an app password for mail apps.';
      break;
    case 'connect':
      hint = 'Check your internet connection and the server names and ports under Advanced. If other apps can get online, your antivirus or firewall may be blocking Mocky Mail.';
      break;
    case 'tls':
      // The certificate belongs to another name: the server name is wrong
      // for this mailbox, which no security setting can fix.
      hint = /NotValidForName/i.test(error.message)
        ? 'That server answers under a different name, so it is probably not the mail server for this address. Enter the incoming and outgoing server names from your email provider under Advanced: server settings.'
        : 'Check the security setting: port 993 (IMAP) and 465 (SMTP) use TLS; 143 and 587 use STARTTLS.';
      break;
    case 'config':
      hint = 'Use your email and an app password instead for now.';
      break;
    case 'keyring':
      hint = 'Mocky Mail stores passwords in the system password store. Make sure it is unlocked and try again.';
      break;
    case 'not_found':
    case 'invalid':
    case 'too_large':
    case 'storage':
    case 'internal':
      break;
  }
  return { title: errorTitle(error.kind), message: error.message, hint };
}

/** The certificate is for another name: the server name is wrong for this mailbox. */
export function isNameMismatch(error: AppError): boolean {
  return error.kind === 'tls' && /NotValidForName/i.test(error.message);
}

/**
 * Add-account wording for a server that could not be reached or trusted. It is
 * shown on the server settings themselves, with the fields open below it.
 */
export function serverErrorCopy(error: AppError): { title: string; message: string } {
  if (error.kind === 'connect') {
    return {
      title: 'We couldn’t reach the mail server',
      message:
        'Check your internet connection and the server names below. If other apps are online, a firewall or antivirus may be blocking Mocky Mail.',
    };
  }
  if (isNameMismatch(error)) {
    return {
      title: 'The server name looks wrong for this address',
      message:
        'The server answered, but its security certificate is for a different name, so we stopped before sending your password. Enter the server names your email provider gives, below.',
    };
  }
  return {
    title: 'Secure connection failed',
    message: 'Check the security setting below: ports 993 and 465 use TLS; 143 and 587 use STARTTLS.',
  };
}
