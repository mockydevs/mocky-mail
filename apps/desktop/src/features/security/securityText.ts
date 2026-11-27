import type { SecurityKey, SecurityKind } from '../../api/types';
import { formatLongDate } from '../../lib/format';

/** "S/MIME" or "OpenPGP": the two ways mail is signed and encrypted. */
export function kindName(kind: SecurityKind): string {
  return kind === 'smime' ? 'S/MIME' : 'OpenPGP';
}

/** What a key of this kind is called: S/MIME has certificates, OpenPGP has keys. */
export function kindNoun(kind: SecurityKind): string {
  return kind === 'smime' ? 'certificate' : 'key';
}

/** "S/MIME certificate" or "OpenPGP key". */
export function kindLabel(kind: SecurityKind): string {
  return `${kindName(kind)} ${kindNoun(kind)}`;
}

const SOURCES: Record<SecurityKey['source'], string> = {
  windows: 'Windows certificate store',
  keychain: 'macOS Keychain',
  pkcs11: 'Smart card or security key',
  gpg: 'GnuPG on this computer',
  android: 'Android',
  mocky: 'Mocky Mail',
};

const KEEPERS: Record<SecurityKey['source'], string> = {
  windows: 'Windows',
  keychain: 'the macOS keychain',
  pkcs11: 'your smart card or security key',
  gpg: 'GnuPG',
  android: 'Android',
  mocky: 'Mocky Mail',
};

/** Who looks after a key kept outside Mocky Mail ("Kept by Windows"). */
export function keptBy(source: SecurityKey['source']): string {
  return KEEPERS[source];
}

/** Where a key is kept, in words. */
export function sourceLabel(source: SecurityKey['source']): string {
  return SOURCES[source];
}

/** A fingerprint in groups of four, the way people read them out to each other. */
export function formatFingerprint(fingerprint: string): string {
  const hex = fingerprint.replace(/[^0-9a-f]/gi, '').toUpperCase();
  if (hex === '') return fingerprint;
  return hex.match(/.{1,4}/g)?.join(' ') ?? hex;
}

export function sameFingerprint(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false;
  return formatFingerprint(a) === formatFingerprint(b);
}

/** The date part of formatLongDate: "Fri 1 Oct 2027". */
function day(seconds: number): string {
  return formatLongDate(seconds).replace(/, \d\d:\d\d$/, '');
}

/** "Expires Fri 1 Oct 2027", "Expired …" or "Doesn't expire", and whether it has. */
export function expiryOf(key: Pick<SecurityKey, 'expires'>, now: number): { text: string; expired: boolean } {
  if (key.expires === null) return { text: 'Doesn’t expire', expired: false };
  const expired = key.expires <= now;
  return { text: `${expired ? 'Expired' : 'Expires'} ${day(key.expires)}`, expired };
}
