import type { Draft, SecurityKind } from '../../api/types';
import { addressOf, parseRecipients, splitRecipients } from './recipients';

/** What the From account can sign and encrypt with. */
export interface AccountSecurity {
  accountId: string;
  smime: boolean;
  openpgp: boolean;
  preferred: SecurityKind;
}

const other = (kind: SecurityKind): SecurityKind => (kind === 'smime' ? 'openpgp' : 'smime');

/**
 * The kind a message is signed and encrypted with: the one chosen for it if
 * the account has that key, else the account's preferred kind, else the
 * other kind when only that one is set up.
 */
export function kindFor(security: AccountSecurity, chosen: SecurityKind | undefined): SecurityKind {
  if (chosen !== undefined && security[chosen]) return chosen;
  if (security[security.preferred]) return security.preferred;
  return security[other(security.preferred)] ? other(security.preferred) : security.preferred;
}

/** Every recipient's address (To, Cc and Bcc), once each. */
export function recipientAddresses(draft: Pick<Draft, 'to' | 'cc' | 'bcc'>): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const address of [draft.to, draft.cc, draft.bcc].flatMap((field) => parseRecipients(field).valid)) {
    const key = address.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(address);
  }
  return out;
}

/** The draft with these recipients taken out of To, Cc and Bcc. */
export function withoutRecipients(draft: Draft, addresses: readonly string[]): Draft {
  const drop = new Set(addresses.map((a) => a.toLowerCase()));
  const keep = (field: string) =>
    splitRecipients(field)
      .filter((entry) => !drop.has(addressOf(entry)?.toLowerCase() ?? ''))
      .join(', ');
  return { ...draft, to: keep(draft.to), cc: keep(draft.cc), bcc: keep(draft.bcc) };
}
