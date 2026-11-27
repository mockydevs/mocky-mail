import { MOCK_MESSAGES } from './mockData';
import { createMockMailApi } from './mockMailApi';
import { MOCK_SECURITY_MESSAGES } from './mockSecurity';
import type { Draft } from '../types';

const draft = (patch: Partial<Draft>): Draft => ({
  accountId: 'work',
  to: 'wanjiru@fitout.example',
  cc: '',
  bcc: '',
  subject: 'Hi',
  bodyText: 'Hello',
  attachments: [],
  replyToMessageId: null,
  inReplyTo: null,
  references: [],
  draftMessageId: null,
  ...patch,
});

describe('mock security backend', () => {
  it('marks signed and encrypted rows only when they are', async () => {
    const api = createMockMailApi({ messages: [...MOCK_MESSAGES, ...MOCK_SECURITY_MESSAGES] });
    const { items } = await api.listMessages({
      accountId: null, folderId: null, role: 'inbox', filter: 'all', search: null, offset: 0, limit: 50,
    });
    const site = items.find((m) => m.id === 'enc-decrypted');
    expect(site).toMatchObject({ signed: true, encrypted: true });
    const plain = items.find((m) => m.id === 'm1');
    expect(plain).toBeDefined();
    expect(plain && 'signed' in plain).toBe(false);
    expect(plain && 'encrypted' in plain).toBe(false);
  });

  it('verifies an OpenPGP signature once its key is trusted', async () => {
    const api = createMockMailApi({ messages: MOCK_SECURITY_MESSAGES });
    expect((await api.getMessage('sig-unverified', false)).signature?.result).toBe('unverifiedSigner');
    await api.setKeyTrusted('pgp-brian', true);
    expect((await api.getMessage('sig-unverified', false)).signature?.result).toBe('verified');
    await api.setKeyTrusted('pgp-brian', false);
    expect((await api.getMessage('sig-unverified', false)).signature?.result).toBe('unverifiedSigner');
  });

  it('knows which recipients can get encrypted mail, per kind', async () => {
    const api = createMockMailApi();
    expect(await api.checkEncryption('work', ['wanjiru@fitout.example', 'Brian@personal.example', 'nobody@else.example'], 'smime')).toEqual({
      kind: 'smime',
      recipients: [
        { address: 'wanjiru@fitout.example', canEncrypt: true },
        { address: 'Brian@personal.example', canEncrypt: false },
        { address: 'nobody@else.example', canEncrypt: false },
      ],
      ownKey: true,
    });
    const pgp = await api.checkEncryption('personal', ['brian@personal.example', 'wanjiru@fitout.example'], 'openpgp');
    expect(pgp.recipients.map((r) => r.canEncrypt)).toEqual([true, false]);
    expect(pgp.ownKey).toBe(true);
    expect((await api.checkEncryption('personal', ['wanjiru@fitout.example'], 'smime')).ownKey).toBe(false);
  });

  it('refuses to send encrypted to someone without a key, or to sign without one', async () => {
    const api = createMockMailApi();
    await expect(api.sendMessage(draft({ to: 'nobody@else.example', encrypt: true }))).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.sendMessage(draft({ accountId: 'personal', sign: true, securityKind: 'smime' }))).rejects.toMatchObject({
      kind: 'config',
    });
    await expect(api.sendMessage(draft({ sign: true, encrypt: true }))).resolves.toMatchObject({ subject: 'Hi' });
  });

  it('imports with the right password only, and dedupes', async () => {
    const api = createMockMailApi();
    const before = (await api.listKeys()).length;
    await expect(api.importKey(null)).rejects.toMatchObject({ kind: 'auth' });
    await expect(api.importKey('nope')).rejects.toMatchObject({ kind: 'auth' });
    expect(await api.importKey('correct horse')).toHaveLength(before + 1);
    expect(await api.importKey('correct horse')).toHaveLength(before + 1);
    expect(await createMockMailApi({ importable: null }).importKey('x')).toBeNull();
  });

  it('keeps account choices to own keys for the account’s address, and forgets removed keys', async () => {
    const api = createMockMailApi();
    await expect(api.setAccountKey('personal', 'smime', 'smime-you-work')).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.setAccountKey('work', 'smime', 'smime-wanjiru')).rejects.toMatchObject({ kind: 'not_found' });
    await expect(api.setAccountKey('work', 'openpgp', 'smime-you-work')).rejects.toMatchObject({ kind: 'invalid' });
    // Kept by Windows: not usable here, and removed there.
    await expect(api.setAccountKey('work', 'smime', 'windows:81D044C32E9A7F15')).rejects.toMatchObject({ kind: 'invalid' });
    await expect(api.removeKey('windows:81D044C32E9A7F15')).rejects.toMatchObject({ kind: 'invalid' });
    await api.removeKey('smime-you-work');
    expect((await api.getAccountKeys('work')).smime).toBeNull();
    await api.setPreferredKind('work', 'openpgp');
    expect((await api.getAccountKeys('work')).preferred).toBe('openpgp');
    expect(await createMockMailApi({ accountKeys: {} }).getAccountKeys('work')).toEqual({ smime: null, openpgp: null, preferred: 'smime' });
  });

  it('exports public keys, and own keys are always trusted', async () => {
    const api = createMockMailApi();
    expect(await api.exportPublicKey('pgp-amina')).toBe('/home/you/Documents/Amina Hassan.asc');
    expect(api.exportedKeys).toEqual(['pgp-amina']);
    await expect(api.setKeyTrusted('pgp-you', false)).rejects.toMatchObject({ kind: 'invalid' });
  });
});
