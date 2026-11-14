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
export function accountErrorCopy(error: AppError, email: string): ErrorCopy {
  const provider = appPasswordProvider(email);
  let hint: string | null = null;
  switch (error.kind) {
    case 'auth':
      hint = provider
        ? `${provider} needs an app password for mail apps, not your normal password. Create one in your ${provider} account security settings and paste it here.`
        : 'Check the password. Some providers need an app password for mail apps.';
      break;
    case 'connect':
      hint = 'Check your internet connection and the server names and ports under Advanced. If other apps can get online, your antivirus or firewall may be blocking Mocky Mail.';
      break;
    case 'tls':
      hint = 'Check the security setting: port 993 (IMAP) and 465 (SMTP) use TLS; 143 and 587 use STARTTLS.';
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
