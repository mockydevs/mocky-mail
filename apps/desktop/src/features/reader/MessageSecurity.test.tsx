import { screen, within } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

import { MOCK_MESSAGES } from '../../api/mock/mockData';
import type { MockOptions } from '../../api/mock/mockMailApi';
import { FINGERPRINTS, MOCK_SECURITY_MESSAGES } from '../../api/mock/mockSecurity';
import type { SignatureCheck } from '../../api/types';
import { formatLongDate } from '../../lib/format';
import { reader, renderApp, rows } from '../../test/renderApp';

const messages = [...MOCK_MESSAGES, ...MOCK_SECURITY_MESSAGES];

/** Open a message by its subject and wait for it in the reader. */
async function open(subject: string, options: MockOptions = {}) {
  const ctx = renderApp({ messages, ...options });
  await ctx.user.click((await rows()).getByRole('button', { name: new RegExp(subject) }));
  const pane = within(reader());
  await pane.findByRole('heading', { name: subject });
  return { ...ctx, pane };
}

const bar = () => within(within(reader()).getByRole('group', { name: 'Signature' }));

describe('signature bar', () => {
  it('shows a verified signature in green, with details', async () => {
    const { user } = await open('Signed contract for the fit-out');
    const group = within(reader()).getByRole('group', { name: 'Signature' });
    expect(group).toHaveClass('sig-bar--ok');
    expect(bar().getByText('Signed by Wanjiru Kamau <wanjiru@fitout.example>')).toBeInTheDocument();
    expect(bar().queryByRole('button', { name: 'Trust this key' })).not.toBeInTheDocument();
    expect(bar().queryByText('The signer isn’t the sender shown.')).not.toBeInTheDocument();

    const toggle = bar().getByRole('button', { name: 'Details' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    const details = within(bar().getByLabelText('Signature details'));
    expect(details.getByText('Wanjiru Kamau <wanjiru@fitout.example>')).toBeInTheDocument();
    expect(details.getByText('Sectigo Email CA')).toBeInTheDocument();
    expect(details.getByText(FINGERPRINTS.wanjiru)).toBeInTheDocument();
    expect(details.getByText(formatLongDate(MOCK_SECURITY_MESSAGES[0]?.signature?.signedAt ?? null))).toBeInTheDocument();
    expect(details.getByText('S/MIME')).toBeInTheDocument();
    expect(details.getAllByRole('term').map((t) => t.textContent)).toEqual([
      'Signed by',
      'Issued by',
      'Fingerprint',
      'Signed at',
      'Kind',
      'What this means',
    ]);
    await user.click(toggle);
    expect(bar().queryByLabelText('Signature details')).not.toBeInTheDocument();
  });

  it('warns in amber when the signer is not verified', async () => {
    await open('My new key');
    expect(within(reader()).getByRole('group', { name: 'Signature' })).toHaveClass('sig-bar--warn');
    expect(bar().getByText('Signed, but the signer isn’t verified')).toBeInTheDocument();
  });

  it('warns in red when the message was changed after signing', async () => {
    const { user } = await open('New bank details for payments');
    expect(within(reader()).getByRole('group', { name: 'Signature' })).toHaveClass('sig-bar--bad');
    expect(bar().getByText('Changed after it was signed — don’t trust it')).toBeInTheDocument();
    await user.click(bar().getByRole('button', { name: 'Details' }));
    expect(
      within(bar().getByLabelText('Signature details')).getByText(
        'The text or attachments are not what was signed. Someone may have changed it on the way.',
      ),
    ).toBeInTheDocument();
  });

  it("says in grey when a signature can't be checked, and why", async () => {
    const { user } = await open('Brand guidelines, draft 2');
    expect(within(reader()).getByRole('group', { name: 'Signature' })).toHaveClass('sig-bar--neutral');
    expect(bar().getByText('Signature can’t be checked')).toBeInTheDocument();
    expect(bar().getByText('The key it was signed with isn’t on this device.')).toBeInTheDocument();
    await user.click(bar().getByRole('button', { name: 'Details' }));
    const details = within(bar().getByLabelText('Signature details'));
    expect(details.getByText('Not known')).toBeInTheDocument();
    expect(details.getByText('OpenPGP')).toBeInTheDocument();
    expect(details.queryByText('Issued by')).not.toBeInTheDocument();
  });

  it('says when the signer is not the sender shown', async () => {
    await open('Statement for October');
    expect(bar().getByText('Signed by Peter Mwangi <peter@client.example>')).toBeInTheDocument();
    expect(bar().getByText('The signer isn’t the sender shown.')).toBeInTheDocument();
    expect(bar().getByText(/It was signed by peter@client\.example\./)).toBeInTheDocument();
  });

  it('shows nothing for unsigned mail', async () => {
    await open('Revised quotation for the Westlands office fit-out');
    expect(within(reader()).queryByRole('group', { name: 'Signature' })).not.toBeInTheDocument();
    expect(within(reader()).queryByRole('note')).not.toBeInTheDocument();
    expect(within(reader()).queryByRole('group', { name: 'Encrypted message' })).not.toBeInTheDocument();
  });
});

describe('trusting an OpenPGP key from the signature bar', () => {
  it('explains comparing the fingerprint, then trusts the key and shows the message as verified', async () => {
    const { user, api } = await open('My new key');
    const trust = vi.spyOn(api, 'setKeyTrusted');
    await user.click(bar().getByRole('button', { name: 'Trust this key' }));
    let dialog = within(screen.getByRole('dialog', { name: 'Trust this key?' }));
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription(
      /Only do this if you’re sure the key belongs to Brian Otieno <brian@personal\.example>\. Ask them for their key’s fingerprint/,
    );
    expect(dialog.getByLabelText('Fingerprint')).toHaveTextContent(FINGERPRINTS.brian);

    // Cancel changes nothing.
    await user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(trust).not.toHaveBeenCalled();

    await user.click(bar().getByRole('button', { name: 'Trust this key' }));
    dialog = within(screen.getByRole('dialog', { name: 'Trust this key?' }));
    await user.click(dialog.getByRole('button', { name: 'Trust this key' }));
    expect(trust).toHaveBeenCalledWith('pgp-brian', true);
    expect(await bar().findByText('Signed by Brian Otieno <brian@personal.example>')).toBeInTheDocument();
    expect(within(reader()).getByRole('group', { name: 'Signature' })).toHaveClass('sig-bar--ok');
    expect(bar().queryByRole('button', { name: 'Trust this key' })).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('says so when the key is no longer on the device', async () => {
    const { user } = await open('My new key', { keys: [] });
    await user.click(bar().getByRole('button', { name: 'Trust this key' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Trust this key?' }));
    await user.click(dialog.getByRole('button', { name: 'Trust this key' }));
    expect(await dialog.findByRole('alert')).toHaveTextContent('This key isn’t on this device any more');
  });

  it('is not offered for S/MIME, whose certificates are vouched for by their issuer', async () => {
    const unverified: SignatureCheck = {
      kind: 'smime',
      result: 'unverifiedSigner',
      signer: { name: 'Peter Mwangi', address: 'peter@client.example', fingerprint: FINGERPRINTS.peter, issuer: 'Self-signed' },
      fromMismatch: false,
      detail: 'The certificate wasn’t issued by anyone this device trusts.',
      signedAt: null,
    };
    const list = messages.map((m) => (m.id === 'sig-changed' ? { ...m, signature: unverified } : m));
    const { user } = await open('New bank details for payments', { messages: list });
    expect(bar().getByText('Signed, but the signer isn’t verified')).toBeInTheDocument();
    expect(bar().queryByRole('button', { name: 'Trust this key' })).not.toBeInTheDocument();
    await user.click(bar().getByRole('button', { name: 'Details' }));
    expect(within(bar().getByLabelText('Signature details')).queryByText('Signed at')).not.toBeInTheDocument();
  });
});

describe('encrypted mail', () => {
  it('notes that a message was decrypted here', async () => {
    await open('Site access codes');
    expect(within(reader()).getByRole('note')).toHaveTextContent('Encrypted — decrypted on this device');
    expect(within(reader()).getByText(/Here are the door codes/)).toBeInTheDocument();
    expect(bar().getByText('Signed by Wanjiru Kamau <wanjiru@fitout.example>')).toBeInTheDocument();
  });

  it("explains a message this device can't open, with a way to Settings › Security", async () => {
    const { user } = await open('Encrypted message');
    const locked = within(within(reader()).getByRole('group', { name: 'Encrypted message' }));
    expect(locked.getByText('This message is encrypted, and this device can’t open it')).toBeInTheDocument();
    expect(locked.getByText('It was encrypted to a key that isn’t on this device.')).toBeInTheDocument();
    expect(locked.getByText(/To read it, add the OpenPGP key it was encrypted to in/)).toBeInTheDocument();
    await user.click(locked.getByRole('button', { name: 'Settings › Security' }));
    const security = await screen.findByRole('region', { name: 'Security' });
    expect(within(screen.getByRole('region', { name: 'Settings' })).getByRole('region', { name: 'Security' })).toBe(security);
    expect(within(security).getByRole('heading', { name: 'Security' })).toHaveFocus();
  });
});

describe('on a phone', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('shows the signature bar and the lock note in the full-screen message', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    const { user } = await open('Site access codes', { phone: true });
    expect(reader()).toHaveClass('reader--phone');
    expect(bar().getByText('Signed by Wanjiru Kamau <wanjiru@fitout.example>')).toBeInTheDocument();
    await user.click(bar().getByRole('button', { name: 'Details' }));
    expect(bar().getByLabelText('Signature details')).toBeInTheDocument();
    expect(within(reader()).getByRole('note')).toHaveTextContent('Encrypted — decrypted on this device');
  });
});
