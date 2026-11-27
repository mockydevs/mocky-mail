import type { SecurityKey, SecurityKind } from '../types';
import { at, message } from './mockData';
import type { MockMessage } from './mockMailApi';

/** Keys an account signs with and encrypts to itself with. */
export interface MockAccountKeys {
  smime: string | null;
  openpgp: string | null;
  preferred: SecurityKind;
}

/** What the file picker hands importKey: the keys in the file, and the password it needs (null: none). */
export interface MockImportable {
  keys: SecurityKey[];
  password: string | null;
}

const key = (k: Partial<SecurityKey> & Pick<SecurityKey, 'id' | 'kind' | 'owner' | 'addresses' | 'fingerprint'>): SecurityKey => ({
  expires: null,
  canSign: true,
  canEncrypt: true,
  isOwn: false,
  source: 'mocky',
  trusted: false,
  ...k,
});

/** Fingerprints as the backend shows them: hex in groups of four. */
export const FINGERPRINTS = {
  youSmime: '3F2A 9C41 7B0E D5A8 1C64 0F93 E27B 4D11 A6C8 52F0',
  youSmimeOld: '81D0 44C3 2E9A 7F15 B0C6 3A28 D917 E4F2 6B05 9A3C',
  youPgp: 'A1B2 C3D4 E5F6 0718 293A 4B5C 6D7E 8F90 1A2B 3C4D',
  wanjiru: '5E7C 12A9 D3F0 6B48 9E21 C07D 4A35 F8B6 2D19 E0C7',
  peter: 'C4E9 0B27 6F13 A8D5 2C70 9E46 B1F8 3D5A 0E92 7C64',
  amina: '9D3E 7A10 C5B2 48F6 E019 3C7A B2D4 6F85 1E0C 93A7',
  brian: '0F6A E3C1 92B7 5D48 A0E6 1C3F 7B92 D5A0 48E1 6C2B',
  unknown: '77E1 0C4A 9B3F D628 15E0 A7C9 3F42 B86D 0E15 9C7A',
} as const;

export const MOCK_KEYS: SecurityKey[] = [
  key({
    id: 'smime-you-work',
    kind: 'smime',
    owner: 'You',
    addresses: ['you@work.example'],
    fingerprint: FINGERPRINTS.youSmime,
    expires: at('2027-10-01T00:00:00Z'),
    isOwn: true,
    source: 'mocky',
    trusted: true,
  }),
  key({
    // Still in the Windows certificate store: listed, but kept (and removed) there.
    id: 'windows:81D044C32E9A7F15',
    kind: 'smime',
    owner: 'You',
    addresses: ['you@work.example'],
    fingerprint: FINGERPRINTS.youSmimeOld,
    expires: at('2025-10-01T00:00:00Z'),
    isOwn: true,
    source: 'windows',
    trusted: true,
  }),
  key({
    id: 'pgp-you',
    kind: 'openpgp',
    owner: 'You',
    addresses: ['you@work.example', 'you@personal.example'],
    fingerprint: FINGERPRINTS.youPgp,
    isOwn: true,
    source: 'mocky',
    trusted: true,
  }),
  key({
    // In GnuPG on this computer: listed, but kept (and removed) there.
    id: 'gpg:5B1E90C47D2A3F68',
    kind: 'openpgp',
    owner: 'You',
    addresses: ['you@personal.example'],
    fingerprint: '5B1E 90C4 7D2A 3F68 0C91 E4B7 2A5D 8F03 C6E1 7B49',
    isOwn: true,
    source: 'gpg',
    trusted: true,
  }),
  key({
    id: 'smime-wanjiru',
    kind: 'smime',
    owner: 'Wanjiru Kamau',
    addresses: ['wanjiru@fitout.example'],
    fingerprint: FINGERPRINTS.wanjiru,
    expires: at('2027-03-15T00:00:00Z'),
    canSign: true,
    trusted: true,
  }),
  key({
    id: 'smime-peter',
    kind: 'smime',
    owner: 'Peter Mwangi',
    addresses: ['peter@client.example'],
    fingerprint: FINGERPRINTS.peter,
    expires: at('2027-06-30T00:00:00Z'),
    trusted: true,
  }),
  key({
    id: 'pgp-amina',
    kind: 'openpgp',
    owner: 'Amina Hassan',
    addresses: ['amina@studio.example'],
    fingerprint: FINGERPRINTS.amina,
    trusted: true,
  }),
  key({
    id: 'pgp-brian',
    kind: 'openpgp',
    owner: 'Brian Otieno',
    addresses: ['brian@personal.example'],
    fingerprint: FINGERPRINTS.brian,
    trusted: false,
  }),
];

export const MOCK_ACCOUNT_KEYS: Record<string, MockAccountKeys> = {
  work: { smime: 'smime-you-work', openpgp: 'pgp-you', preferred: 'smime' },
  personal: { smime: null, openpgp: 'pgp-you', preferred: 'openpgp' },
};

/** The sample file the picker returns: a .p12 for the personal address, locked with "correct horse". */
export const MOCK_IMPORTABLE: MockImportable = {
  keys: [
    key({
      id: 'smime-you-personal',
      kind: 'smime',
      owner: 'You',
      addresses: ['you@personal.example'],
      fingerprint: '2B8E 61F0 C947 3AD5 0E18 B6C2 7F94 1D03 E5A9 6C27',
      expires: at('2028-01-31T00:00:00Z'),
      isOwn: true,
      source: 'mocky',
      trusted: true,
    }),
  ],
  password: 'correct horse',
};

const para = (...p: string[]) => p.join('\n\n');

/**
 * Signed and encrypted samples: every result the reader can show. They are
 * not in the default data (other tests count messages); the dev preview and
 * the security tests add them.
 */
export const MOCK_SECURITY_MESSAGES: MockMessage[] = [
  message({
    id: 'sig-verified', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
    subject: 'Signed contract for the fit-out',
    preview: 'The signed contract is attached. Work starts on the 24th.',
    date: at('2026-11-09T09:10:00Z'), signed: true,
    text: para('Hi,', 'The signed contract is attached. Work starts on the 24th.', 'Wanjiru'),
    signature: {
      kind: 'smime', result: 'verified',
      signer: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example', fingerprint: FINGERPRINTS.wanjiru, issuer: 'Sectigo Email CA' },
      fromMismatch: false,
      detail: 'The certificate was issued by Sectigo Email CA, which this device trusts, and the message is unchanged.',
      signedAt: at('2026-11-09T09:09:41Z'),
    },
  }),
  message({
    id: 'sig-unverified', accountId: 'personal', folderId: 'personal:inbox',
    from: { name: 'Brian Otieno', address: 'brian@personal.example' },
    subject: 'My new key',
    preview: 'I have started signing my mail. My fingerprint ends in 48E1 6C2B.',
    date: at('2026-11-09T08:00:00Z'), signed: true,
    text: para('Hey,', 'I have started signing my mail. My fingerprint ends in 48E1 6C2B, ask me if you want to check the rest.', 'Brian'),
    signature: {
      kind: 'openpgp', result: 'unverifiedSigner',
      signer: { name: 'Brian Otieno', address: 'brian@personal.example', fingerprint: FINGERPRINTS.brian, issuer: null },
      fromMismatch: false,
      detail: 'The message is unchanged, but you haven’t said that this key really belongs to Brian Otieno.',
      signedAt: at('2026-11-09T07:59:50Z'),
    },
  }),
  message({
    id: 'sig-changed', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Peter Mwangi', address: 'peter@client.example' },
    subject: 'New bank details for payments',
    preview: 'Please send future payments to the account below.',
    date: at('2026-11-08T15:30:00Z'), signed: true,
    text: para('Hello,', 'Please send future payments to the account below. Our old account is closed.', 'Peter'),
    signature: {
      kind: 'smime', result: 'changed',
      signer: { name: 'Peter Mwangi', address: 'peter@client.example', fingerprint: FINGERPRINTS.peter, issuer: 'DigiCert Email CA' },
      fromMismatch: false,
      detail: 'The text or attachments are not what was signed. Someone may have changed it on the way.',
      signedAt: at('2026-11-08T15:29:12Z'),
    },
  }),
  message({
    id: 'sig-unknown', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Amina Hassan', address: 'amina@studio.example' },
    subject: 'Brand guidelines, draft 2',
    preview: 'Draft 2 of the guidelines, with the colour pages redone.',
    date: at('2026-11-08T10:05:00Z'), signed: true,
    text: para('Hi,', 'Draft 2 of the guidelines, with the colour pages redone.', 'Amina'),
    signature: {
      kind: 'openpgp', result: 'unknown',
      signer: { name: null, address: null, fingerprint: FINGERPRINTS.unknown, issuer: null },
      fromMismatch: false,
      detail: 'The key it was signed with isn’t on this device.',
      signedAt: at('2026-11-08T10:04:30Z'),
    },
  }),
  message({
    id: 'sig-mismatch', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Client accounts', address: 'accounts@client.example' },
    subject: 'Statement for October',
    preview: 'Your statement for October is attached.',
    date: at('2026-11-07T12:00:00Z'), signed: true,
    text: para('Hello,', 'Your statement for October is attached.'),
    signature: {
      kind: 'smime', result: 'verified',
      signer: { name: 'Peter Mwangi', address: 'peter@client.example', fingerprint: FINGERPRINTS.peter, issuer: 'DigiCert Email CA' },
      fromMismatch: true,
      detail: 'The certificate was issued by DigiCert Email CA, which this device trusts, and the message is unchanged.',
      signedAt: at('2026-11-07T11:59:30Z'),
    },
  }),
  message({
    id: 'enc-decrypted', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
    subject: 'Site access codes',
    preview: 'Here are the door codes for the site from Monday.',
    date: at('2026-11-07T08:20:00Z'), signed: true, encrypted: true,
    text: para('Hi,', 'Here are the door codes for the site from Monday. Please don’t forward them.', 'Wanjiru'),
    encryption: { kind: 'smime', state: 'decrypted', detail: null },
    signature: {
      kind: 'smime', result: 'verified',
      signer: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example', fingerprint: FINGERPRINTS.wanjiru, issuer: 'Sectigo Email CA' },
      fromMismatch: false,
      detail: 'The certificate was issued by Sectigo Email CA, which this device trusts, and the message is unchanged.',
      signedAt: at('2026-11-07T08:19:40Z'),
    },
  }),
  message({
    id: 'enc-locked', accountId: 'personal', folderId: 'personal:inbox',
    from: { name: 'Grace Njeri', address: 'grace@personal.example' },
    subject: 'Encrypted message',
    preview: '',
    date: at('2026-11-06T18:45:00Z'), encrypted: true,
    encryption: {
      kind: 'openpgp', state: 'locked',
      detail: 'It was encrypted to a key that isn’t on this device.',
    },
  }),
];
