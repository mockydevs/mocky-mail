import { screen, waitFor, within, type BoundFunctions, type queries } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

import { MOCK_MESSAGES } from '../../api/mock/mockData';
import type { MockOptions } from '../../api/mock/mockMailApi';
import { FINGERPRINTS, MOCK_IMPORTABLE, MOCK_SECURITY_MESSAGES } from '../../api/mock/mockSecurity';
import type { SecurityKey } from '../../api/types';
import { reader, renderApp, rows } from '../../test/renderApp';

async function openSecurity(options: MockOptions = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  const region = screen.getByRole('region', { name: 'Security' });
  const security = within(region);
  // The keys are loaded: listed, or none yet.
  await waitFor(() => {
    expect(security.queryByRole('list', { name: 'Your keys and certificates' }) ?? security.queryByText(/^None yet\. Import one/)).not.toBeNull();
  });
  return { ...ctx, region, security };
}

const ownList = (security: BoundFunctions<typeof queries>) => within(security.getByRole('list', { name: 'Your keys and certificates' }));
const othersList = (security: BoundFunctions<typeof queries>) =>
  within(security.getByRole('list', { name: 'Other people’s keys and certificates' }));
const account = (security: BoundFunctions<typeof queries>, name: string) => within(security.getByRole('group', { name }));

describe('Settings › Security: your keys and certificates', () => {
  it('lists them with kind, owner, addresses, expiry and where they are kept', async () => {
    const { security } = await openSecurity();
    const own = ownList(security);
    expect(own.getAllByRole('listitem').map((li) => li.getAttribute('aria-label'))).toEqual([
      'S/MIME certificate for you@work.example',
      'S/MIME certificate for you@work.example (expired)',
      'OpenPGP key for you@work.example',
      'OpenPGP key for you@personal.example',
    ]);
    const current = within(own.getByRole('listitem', { name: 'S/MIME certificate for you@work.example' }));
    expect(current.getByText('You')).toBeInTheDocument();
    expect(current.getByText('· S/MIME certificate')).toBeInTheDocument();
    expect(current.getByText('For you@work.example')).toBeInTheDocument();
    expect(current.getByText(/^Expires /)).not.toHaveClass('key-row__expiry--expired');
    expect(current.getByText(/Kept in Mocky Mail/)).toBeInTheDocument();
    expect(current.getByRole('button', { name: 'Remove S/MIME certificate for you@work.example' })).toBeInTheDocument();

    const expired = within(own.getByRole('listitem', { name: 'S/MIME certificate for you@work.example (expired)' }));
    expect(expired.getByText(/^Expired /)).toHaveClass('key-row__expiry--expired');
    // Kept by Windows: removed there, not here.
    expect(expired.getByText(/Kept in Windows certificate store/)).toBeInTheDocument();
    expect(expired.getByText('Kept by Windows; remove it there.')).toBeInTheDocument();
    expect(expired.queryByRole('button', { name: /^Remove/ })).not.toBeInTheDocument();
    const gpg = within(own.getByRole('listitem', { name: 'OpenPGP key for you@personal.example' }));
    expect(gpg.getByText('Kept by GnuPG; remove it there.')).toBeInTheDocument();

    const pgp = within(own.getByRole('listitem', { name: 'OpenPGP key for you@work.example' }));
    expect(pgp.getByText('For you@work.example, you@personal.example')).toBeInTheDocument();
    expect(pgp.getByText('Doesn’t expire')).toBeInTheDocument();
    expect(pgp.getByText(/Kept in Mocky Mail/)).toBeInTheDocument();
  });

  it('says so when there are none', async () => {
    const { security } = await openSecurity({ keys: [] });
    expect(security.getByText('None yet. Import one to sign and encrypt your mail.')).toBeInTheDocument();
    expect(security.getByText(/They’re added when people send you signed mail/)).toBeInTheDocument();
  });

  it('exports a public key to share', async () => {
    const { user, api, security } = await openSecurity();
    await user.click(security.getByRole('button', { name: 'Export public key: S/MIME certificate for you@work.example' }));
    expect(await security.findByRole('status')).toHaveTextContent('Saved: /home/you/Documents/You.cer');
    expect(api.exportedKeys).toEqual(['smime-you-work']);
  });

  it('says nothing when saving the public key is cancelled', async () => {
    const { user, api, security } = await openSecurity({ exportCancelled: true });
    const exported = vi.spyOn(api, 'exportPublicKey');
    await user.click(security.getByRole('button', { name: 'Export public key: OpenPGP key for you@work.example' }));
    await waitFor(() => {
      expect(exported).toHaveBeenCalledWith('pgp-you');
    });
    expect(security.queryByRole('status')).not.toBeInTheDocument();
    expect(security.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('removes one only after explaining that old mail encrypted to it can’t be opened', async () => {
    const { user, api, security } = await openSecurity();
    const remove = vi.spyOn(api, 'removeKey');
    await user.click(security.getByRole('button', { name: 'Remove OpenPGP key for you@work.example' }));
    let dialog = within(screen.getByRole('dialog', { name: 'Remove this key?' }));
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription(
      /Mail that was encrypted to it can’t be opened on this device any more, including messages you already have/,
    );
    await user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(remove).not.toHaveBeenCalled();
    expect(ownList(security).getByRole('listitem', { name: 'OpenPGP key for you@work.example' })).toBeInTheDocument();

    await user.click(security.getByRole('button', { name: 'Remove OpenPGP key for you@work.example' }));
    dialog = within(screen.getByRole('dialog', { name: 'Remove this key?' }));
    await user.click(dialog.getByRole('button', { name: 'Remove key' }));
    expect(remove).toHaveBeenCalledWith('pgp-you');
    expect(await security.findByRole('status')).toHaveTextContent('Removed the OpenPGP key for you@work.example.');
    expect(ownList(security).queryByRole('listitem', { name: 'OpenPGP key for you@work.example' })).not.toBeInTheDocument();
    // The accounts that used it now use none.
    await waitFor(() => {
      expect(account(security, 'Work (you@work.example)').getByRole('combobox', { name: 'OpenPGP key' })).toHaveValue('');
    });
    expect(account(security, 'Personal (you@personal.example)').getByRole('combobox', { name: 'OpenPGP key' })).toHaveValue('');
  });
});

describe('Settings › Security: importing', () => {
  it('asks for the file’s password, uses it once and never keeps it', async () => {
    const { user, api, security } = await openSecurity();
    const importKey = vi.spyOn(api, 'importKey');
    await user.click(security.getByRole('button', { name: 'Import key or certificate…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Import a key or certificate' }));
    const password = dialog.getByLabelText('The file’s password, if it has one');
    expect(password).toHaveAttribute('type', 'password');
    expect(password).toHaveAttribute('autocomplete', 'off');
    expect(password).toHaveAccessibleDescription(/used once to open the file and isn’t kept/);

    await user.type(password, 'wrong');
    await user.click(dialog.getByRole('button', { name: 'Choose file…' }));
    expect(importKey).toHaveBeenLastCalledWith('wrong');
    expect(await dialog.findByRole('alert')).toHaveTextContent('That password didn’t open the file. Check it and try again.');
    expect(password).toHaveValue('');
    expect(password).toHaveAttribute('aria-invalid', 'true');

    await user.type(password, MOCK_IMPORTABLE.password ?? '');
    await user.click(dialog.getByRole('button', { name: 'Choose file…' }));
    expect(importKey).toHaveBeenLastCalledWith('correct horse');
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    expect(screen.queryByDisplayValue('correct horse')).not.toBeInTheDocument();
    expect(await security.findByRole('status')).toHaveTextContent('Added: S/MIME certificate for you@personal.example.');
    expect(ownList(security).getByRole('listitem', { name: 'S/MIME certificate for you@personal.example' })).toBeInTheDocument();
    // It can now be chosen for the personal account.
    const smime = account(security, 'Personal (you@personal.example)').getByRole('combobox', { name: 'S/MIME certificate' });
    expect(within(smime).getAllByRole('option')).toHaveLength(2);
  });

  it('asks for a password when the file needs one and none was given', async () => {
    const { user, api, security } = await openSecurity();
    const importKey = vi.spyOn(api, 'importKey');
    await user.click(security.getByRole('button', { name: 'Import key or certificate…' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Import a key or certificate' }));
    await user.click(dialog.getByRole('button', { name: 'Choose file…' }));
    expect(importKey).toHaveBeenCalledWith(null);
    expect(await dialog.findByRole('alert')).toHaveTextContent('This file is protected with a password.');
  });

  it('imports a file without a password', async () => {
    const pgp: SecurityKey = {
      id: 'pgp-grace',
      kind: 'openpgp',
      owner: 'Grace Njeri',
      addresses: ['grace@personal.example'],
      fingerprint: '1111 2222 3333 4444 5555 6666 7777 8888 9999 0000',
      expires: null,
      canSign: true,
      canEncrypt: true,
      isOwn: false,
      source: 'mocky',
      trusted: false,
    };
    const { user, api, security } = await openSecurity({ importable: { keys: [pgp], password: null } });
    const importKey = vi.spyOn(api, 'importKey');
    await user.click(security.getByRole('button', { name: 'Import key or certificate…' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Choose file…' }));
    expect(importKey).toHaveBeenCalledWith(null);
    expect(await security.findByRole('status')).toHaveTextContent('Added: OpenPGP key for Grace Njeri.');
    expect(othersList(security).getByRole('listitem', { name: 'OpenPGP key for Grace Njeri' })).toBeInTheDocument();
  });

  it('does nothing when the file picker is cancelled', async () => {
    const { user, api, security } = await openSecurity({ importable: null });
    const before = (await api.listKeys()).length;
    await user.click(security.getByRole('button', { name: 'Import key or certificate…' }));
    await user.type(screen.getByLabelText('The file’s password, if it has one'), 'secret');
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Choose file…' }));
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    expect(security.queryByRole('status')).not.toBeInTheDocument();
    expect(await api.listKeys()).toHaveLength(before);
  });
});

describe('Settings › Security: each account', () => {
  it('chooses the certificate and key an account uses, from the ones for its address', async () => {
    const { user, api, security } = await openSecurity();
    const setKey = vi.spyOn(api, 'setAccountKey');
    const work = account(security, 'Work (you@work.example)');
    const smime = await work.findByRole('combobox', { name: 'S/MIME certificate' });
    expect(smime).toHaveValue('smime-you-work');
    expect(within(smime).getAllByRole('option').map((o) => o.textContent)).toEqual([
      'None',
      expect.stringMatching(/^You · Expires .* · …A6C8 52F0$/) as string,
    ]);
    // The expired one is kept by Windows: it can't be chosen, and it says how to use it.
    expect(
      work.getByText(/S\/MIME certificate for you@work\.example \(expired\) is kept by Windows, so it can’t be chosen here\. To sign or open encrypted mail with it, export it from there/),
    ).toBeInTheDocument();
    expect(work.getByRole('combobox', { name: 'OpenPGP key' })).toHaveValue('pgp-you');

    await user.selectOptions(smime, 'None');
    expect(setKey).toHaveBeenCalledWith('work', 'smime', null);
    expect((await api.getAccountKeys('work')).smime).toBeNull();

    // Personal has no S/MIME certificate of its own: only None.
    const personal = account(security, 'Personal (you@personal.example)');
    expect(within(personal.getByRole('combobox', { name: 'S/MIME certificate' })).getAllByRole('option')).toHaveLength(1);
    // Its GnuPG key isn't offered; the one in Mocky Mail is.
    expect(
      within(personal.getByRole('combobox', { name: 'OpenPGP key' }))
        .getAllByRole('option')
        .map((o) => o.getAttribute('value')),
    ).toEqual(['', 'pgp-you']);
    expect(personal.getByText(/OpenPGP key for you@personal\.example is kept by GnuPG/)).toBeInTheDocument();
  });

  it('sets which kind an account uses first', async () => {
    const { user, api, security } = await openSecurity();
    const prefer = vi.spyOn(api, 'setPreferredKind');
    const work = account(security, 'Work (you@work.example)');
    const first = await work.findByRole('combobox', { name: 'Use first' });
    expect(first).toHaveDisplayValue('S/MIME');
    await user.selectOptions(first, 'OpenPGP');
    expect(prefer).toHaveBeenCalledWith('work', 'openpgp');
    expect((await api.getAccountKeys('work')).preferred).toBe('openpgp');
  });

  it('puts a choice back when the backend refuses it', async () => {
    const { user, api, security } = await openSecurity();
    api.failNext('setAccountKey', { kind: 'invalid', message: 'That certificate isn’t for you@work.example.' });
    const smime = await account(security, 'Work (you@work.example)').findByRole('combobox', { name: 'S/MIME certificate' });
    await user.selectOptions(smime, 'None');
    expect(await security.findByRole('alert')).toHaveTextContent('That certificate isn’t for you@work.example.');
    expect(smime).toHaveValue('smime-you-work');
  });
});

describe('Settings › Security: defaults', () => {
  it('turns signing and encrypting new messages on by default', async () => {
    const { user, api, security } = await openSecurity();
    expect(security.getByText(/Search doesn’t look inside encrypted messages\./)).toBeInTheDocument();
    const sign = security.getByRole('checkbox', { name: 'Sign new messages by default' });
    const encrypt = security.getByRole('checkbox', { name: 'Encrypt when possible' });
    expect(sign).not.toBeChecked();
    expect(encrypt).not.toBeChecked();
    await user.click(sign);
    await user.click(encrypt);
    await waitFor(async () => {
      expect(await api.getSettings()).toMatchObject({ signByDefault: true, encryptWhenPossible: true });
    });
    expect(sign).toBeChecked();
    expect(encrypt).toBeChecked();
  });
});

describe('Settings › Security: other people’s keys', () => {
  it('lists them with their fingerprints', async () => {
    const { security } = await openSecurity();
    const others = othersList(security);
    expect(others.getAllByRole('listitem').map((li) => li.getAttribute('aria-label'))).toEqual([
      'S/MIME certificate for Wanjiru Kamau',
      'S/MIME certificate for Peter Mwangi',
      'OpenPGP key for Amina Hassan',
      'OpenPGP key for Brian Otieno',
    ]);
    const brian = within(others.getByRole('listitem', { name: 'OpenPGP key for Brian Otieno' }));
    expect(brian.getByText('For brian@personal.example')).toBeInTheDocument();
    expect(brian.getByText(FINGERPRINTS.brian)).toBeInTheDocument();
    const peter = within(others.getByRole('listitem', { name: 'S/MIME certificate for Peter Mwangi' }));
    expect(peter.getByText('Certificates are vouched for by whoever issued them.')).toBeInTheDocument();
    expect(peter.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('trusts a key only after showing its fingerprint to compare, and stops trusting at once', async () => {
    const { user, api, security } = await openSecurity();
    const trust = vi.spyOn(api, 'setKeyTrusted');
    const brian = security.getByRole('checkbox', { name: 'I’ve checked this key belongs to Brian Otieno' });
    expect(brian).not.toBeChecked();
    await user.click(brian);
    let dialog = within(screen.getByRole('dialog', { name: 'Trust this key?' }));
    expect(dialog.getByLabelText('Fingerprint')).toHaveTextContent(FINGERPRINTS.brian);
    await user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(trust).not.toHaveBeenCalled();
    expect(brian).not.toBeChecked();

    await user.click(brian);
    dialog = within(screen.getByRole('dialog', { name: 'Trust this key?' }));
    await user.click(dialog.getByRole('button', { name: 'Trust this key' }));
    expect(trust).toHaveBeenCalledWith('pgp-brian', true);
    await waitFor(() => {
      expect(security.getByRole('checkbox', { name: 'I’ve checked this key belongs to Brian Otieno' })).toBeChecked();
    });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    await user.click(security.getByRole('checkbox', { name: 'I’ve checked this key belongs to Amina Hassan' }));
    expect(trust).toHaveBeenLastCalledWith('pgp-amina', false);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() => {
      expect(security.getByRole('checkbox', { name: 'I’ve checked this key belongs to Amina Hassan' })).not.toBeChecked();
    });
  });

  it('removes one after saying what that means', async () => {
    const { user, security } = await openSecurity();
    await user.click(security.getByRole('button', { name: 'Remove S/MIME certificate for Peter Mwangi' }));
    expect(screen.getByRole('dialog', { name: 'Remove this certificate?' })).toHaveAccessibleDescription(
      'You won’t be able to encrypt mail to Peter Mwangi or check their signatures until their certificate is added again.',
    );
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Remove certificate' }));
    await waitFor(() => {
      expect(othersList(security).queryByRole('listitem', { name: 'S/MIME certificate for Peter Mwangi' })).not.toBeInTheDocument();
    });
  });
});

describe('Settings › Security on a phone', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('opens from a message that can’t be read, with everything in one column', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    const { user } = renderApp({ phone: true, messages: [...MOCK_MESSAGES, ...MOCK_SECURITY_MESSAGES] });
    await user.click((await rows()).getByRole('button', { name: /Encrypted message/ }));
    await user.click(await within(reader()).findByRole('button', { name: 'Settings › Security' }));
    const security = within(await screen.findByRole('region', { name: 'Security' }));
    expect(security.getByRole('heading', { name: 'Security' })).toHaveFocus();
    expect(await security.findByRole('list', { name: 'Your keys and certificates' })).toBeInTheDocument();
    expect(security.getByRole('button', { name: 'Import key or certificate…' })).toBeInTheDocument();
    expect(security.getByRole('checkbox', { name: 'Sign new messages by default' })).toBeInTheDocument();
    await user.click(security.getByRole('button', { name: 'Import key or certificate…' }));
    expect(screen.getByRole('dialog', { name: 'Import a key or certificate' })).toBeInTheDocument();
  });
});
