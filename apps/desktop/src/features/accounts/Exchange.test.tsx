import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import type { Account, ExchangeAccountInput } from '../../api/types';
import { deferred, renderApp, rows } from '../../test/renderApp';
import { checkEwsUrl } from './ExchangeForm';

const exchangeForm = () => within(screen.getByRole('form', { name: 'Add an Exchange Server account' }));

async function openExchange(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp({ accounts: [], ...options });
  await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
  const main = within(screen.getByRole('form', { name: 'Add an email account' }));
  await ctx.user.click(main.getByRole('button', { name: 'Exchange Server' }));
  return ctx;
}

describe('adding an Exchange Server account', () => {
  it('finds the server from the address, says so while it looks, and opens the mailbox', async () => {
    const { user, api } = await openExchange();
    const form = exchangeForm();
    const original = api.addExchangeAccount.bind(api);
    const gate = deferred();
    const spy = vi.spyOn(api, 'addExchangeAccount').mockImplementation(async (input: ExchangeAccountInput) => {
      await gate.promise;
      return original(input);
    });
    await user.type(form.getByLabelText('Email address'), 'wanjiru@corp.example');
    await user.type(form.getByLabelText('Your name (optional)'), 'Wanjiru');
    await user.type(form.getByLabelText('Password'), 'secret');
    await user.type(form.getByLabelText(/^Sign-in name/), 'CORP\\wanjiru');
    await user.click(form.getByRole('button', { name: 'Add Exchange account' }));

    expect(form.getByRole('status')).toHaveTextContent('Finding your Exchange server…');
    expect(form.getByRole('button', { name: 'Checking sign-in…' })).toBeDisabled();
    expect(spy).toHaveBeenCalledWith({
      email: 'wanjiru@corp.example',
      displayName: 'Wanjiru',
      username: 'CORP\\wanjiru',
      password: 'secret',
      ewsUrl: null,
    });
    // The password is never kept in the form.
    expect(form.getByLabelText('Password')).toHaveValue('');
    gate.resolve();

    expect(await screen.findByRole('list', { name: 'All accounts' })).toBeInTheDocument();
    const added = (await api.listAccounts()).find((a) => a.email === 'wanjiru@corp.example');
    expect(added).toMatchObject({ auth: 'exchange', protocol: 'ews', imap: { host: 'mail.corp.example', port: 443 } });
  });

  it('uses a server address typed by hand', async () => {
    const { user, api } = await openExchange();
    const form = exchangeForm();
    const spy = vi.spyOn(api, 'addExchangeAccount').mockReturnValue(new Promise(() => undefined));
    await user.type(form.getByLabelText('Email address'), 'wanjiru@corp.example');
    await user.type(form.getByLabelText('Password'), 'secret');
    expect(form.queryByLabelText('Server address')).not.toBeInTheDocument();
    const disclosure = form.getByRole('button', { name: 'Enter the server address myself' });
    expect(disclosure).toHaveAttribute('aria-expanded', 'false');
    await user.click(disclosure);
    expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    await user.type(form.getByLabelText('Server address'), 'https://owa.corp.example/EWS/Exchange.asmx');
    await user.click(form.getByRole('button', { name: 'Add Exchange account' }));
    expect(spy).toHaveBeenCalledWith(
      expect.objectContaining({ ewsUrl: 'https://owa.corp.example/EWS/Exchange.asmx', username: '' }),
    );
    expect(form.getByRole('status')).toHaveTextContent('Connecting to owa.corp.example…');
  });

  it('accepts only https server addresses', async () => {
    const { user, api } = await openExchange();
    const form = exchangeForm();
    const spy = vi.spyOn(api, 'addExchangeAccount');
    await user.type(form.getByLabelText('Email address'), 'wanjiru@corp.example');
    await user.click(form.getByRole('button', { name: 'Enter the server address myself' }));
    await user.type(form.getByLabelText('Server address'), 'http://owa.corp.example/EWS/Exchange.asmx');
    await user.type(form.getByLabelText('Password'), 'secret');
    await user.click(form.getByRole('button', { name: 'Add Exchange account' }));
    expect(form.getByRole('alert')).toHaveTextContent('Use an address that starts with https://');
    expect(form.getByLabelText('Server address')).toHaveAttribute('aria-invalid', 'true');
    expect(spy).not.toHaveBeenCalled();

    expect(checkEwsUrl('')).toEqual({ url: null });
    expect(checkEwsUrl(' mail.corp.example ')).toEqual({ url: 'mail.corp.example' });
    expect(checkEwsUrl('ftp://mail.corp.example')).toHaveProperty('error');
    expect(checkEwsUrl('not a server')).toHaveProperty('error');
  });

  it('explains a refused sign-in and a server that could not be found', async () => {
    const { user } = await openExchange();
    const form = exchangeForm();
    await user.type(form.getByLabelText('Email address'), 'wanjiru@corp.example');
    await user.type(form.getByLabelText('Password'), 'wrong');
    await user.click(form.getByRole('button', { name: 'Add Exchange account' }));
    expect(await form.findByRole('alert')).toHaveTextContent('That sign-in wasn’t accepted');
    expect(form.getByLabelText('Password')).toHaveAttribute('aria-invalid', 'true');

    await user.clear(form.getByLabelText('Email address'));
    await user.type(form.getByLabelText('Email address'), 'wanjiru@corp.invalid');
    await user.type(form.getByLabelText('Password'), 'secret');
    await user.click(form.getByRole('button', { name: 'Add Exchange account' }));
    expect(await form.findByText('We couldn’t find your Exchange server')).toBeInTheDocument();
    // The address field opens, to type the server in.
    expect(form.getByLabelText('Server address')).toBeInTheDocument();
    expect(form.getByRole('button', { name: 'Add Exchange account' })).toBeEnabled();
  });

  it('goes back to the other kinds of account with the address kept', async () => {
    const { user } = await openExchange();
    await user.type(exchangeForm().getByLabelText('Email address'), 'don@corp.example');
    await user.click(exchangeForm().getByRole('button', { name: 'Back' }));
    const main = within(screen.getByRole('form', { name: 'Add an email account' }));
    await user.click(main.getByRole('button', { name: 'Exchange Server' }));
    expect(exchangeForm().getByLabelText('Email address')).toHaveValue('don@corp.example');
  });
});

describe('an Exchange account in Settings › Accounts', () => {
  it('says Exchange, with no IMAP or SMTP servers, and changes its password', async () => {
    const [work] = MOCK_ACCOUNTS;
    if (!work) throw new Error('mock accounts missing');
    const exchange: Account = {
      ...work,
      id: 'corp',
      email: 'you@corp.example',
      displayName: 'Corp',
      auth: 'exchange',
      protocol: 'ews',
      imap: { host: 'mail.corp.example', port: 443, security: 'tls' },
      smtp: { host: 'mail.corp.example', port: 443, security: 'tls' },
      sortOrder: 1,
    };
    const { user, api } = renderApp({ accounts: [work, exchange] });
    await rows();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const region = within(screen.getByRole('region', { name: 'Settings' }));
    const row = within(region.getByText('you@corp.example').closest('li') as HTMLElement);
    expect(row.getByText('Exchange, server mail.corp.example')).toBeInTheDocument();
    expect(row.queryByText(/IMAP|SMTP/)).not.toBeInTheDocument();
    expect(row.queryByRole('button', { name: 'Sign in again' })).not.toBeInTheDocument();
    const update = vi.spyOn(api, 'updateAccount');
    await user.click(row.getByRole('button', { name: 'Change password' }));
    await user.type(row.getByLabelText('New password or app password'), 'new-secret');
    await user.click(row.getByRole('button', { name: 'Update password' }));
    expect(update).toHaveBeenCalledWith('corp', { password: 'new-secret' });
    expect(await row.findByText('Password updated.')).toBeInTheDocument();
  });
});
